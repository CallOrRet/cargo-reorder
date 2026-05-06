//! CLI entry point. Default file discovery, build-tree filtering, and
//! output format mirror `cargo fmt` / `rustfmt`.

use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use walkdir::WalkDir;

use cargo_reorder::ReorderError;
use cargo_reorder::diagnostic::{
    Color as DiagColor, format_parse_error_colored, write_diff_colored,
};
use cargo_reorder::discover::{DiscoverOptions, discover};
use cargo_reorder::{Config, reorder_source_with, reorder_source_with_path};

#[derive(ValueEnum, Clone, Copy, Debug)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

enum Outcome {
    Changed,
    Unchanged,
    ParseError,
}

#[derive(Parser, Debug)]
#[command(
    name = "cargo-reorder",
    about = "Reorder top-level items in Rust source files (use/mod/const/struct/...).",
    version,
    after_help = "\
DEFAULT FILE DISCOVERY:
  With no PATHS argument and a tty stdin, cargo-reorder behaves like
  `cargo fmt`: it runs `cargo metadata --no-deps`, takes the current
  package's targets, and walks each target's module tree.

  With a piped stdin (no PATHS), source is read from stdin and written
  to stdout (filter mode).

  With PATHS, the listed files / directories are processed directly.

OUTPUT:
  Default mode is silent on success; pass `-v` to log every rewrite.
  `--check` prints a unified diff per file in rustfmt format and exits 1
  if anything would change. Parse errors are rendered rustc-style.

EXAMPLES:
  cargo-reorder                     reorder current package's source tree
  cargo-reorder --all               reorder every workspace member
  cargo-reorder -p foo -p bar       reorder only the named packages
  cargo-reorder --check             CI mode: diff + exit 1 if changes
  cargo-reorder --no-mod-before-use put use declarations before mod
  cargo-reorder src/lib.rs          reorder a single file
  cargo-reorder < a.rs > b.rs       filter mode (piped stdin)
"
)]
struct Cli {
    /// Files or directories to process. If omitted, files are discovered via
    /// `cargo metadata`.
    paths: Vec<PathBuf>,

    /// Process every workspace member (matches `cargo fmt --all`).
    #[arg(long, conflicts_with = "package")]
    all: bool,

    /// Process only the named package(s) (matches `cargo fmt -p NAME`).
    #[arg(short = 'p', long = "package", value_name = "NAME")]
    package: Vec<String>,

    /// Path to the `Cargo.toml` to use for metadata discovery.
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<PathBuf>,

    /// Print a unified diff for every file that would change and exit 1
    /// if any do. Does not modify files.
    #[arg(long)]
    check: bool,

    /// Print the reordered output to stdout instead of rewriting in place.
    #[arg(long, conflicts_with = "check")]
    stdout: bool,

    /// Disable putting `mod foo;` before `use ...;` (i.e., switch to
    /// use-first). Default is mod-first because the pair-majority
    /// sample of 21 real-world crates leans 12-vs-7 toward mod-first
    /// (see README). Pass this flag if your project is on the
    /// use-first side (notably `regex`, `ripgrep`, `cargo`, `chrono`,
    /// `tracing`).
    #[arg(long)]
    no_mod_before_use: bool,

    /// Disable splitting `use` items into std / external / crate groups.
    #[arg(long)]
    no_import_groups: bool,

    /// Disable anchoring `impl` blocks to their target type.
    #[arg(long)]
    no_impl_grouping: bool,

    /// Do not force `#[cfg(test)] mod ...` to the end of the file.
    #[arg(long)]
    no_tests_last: bool,

    /// Disable preserving `pub mod` / `mod` source order. With this
    /// on, `pub mod` items are sorted before private `mod` items with
    /// a blank line between the two groups. Default is to preserve
    /// source order — empirically interleaved is the modal pattern
    /// (11/19 projects in the README sample).
    #[arg(long)]
    no_preserve_mod_order: bool,

    /// Disable putting `trait` ahead of `enum` / `struct` / `union`
    /// (i.e., switch to struct-first). Default is trait-first — in
    /// the 21-project sample 14/20 lean trait-first under
    /// pair-majority. Pass this flag for projects that consistently
    /// put structs / enums first.
    #[arg(long)]
    no_trait_before_struct: bool,

    /// Skip recursing into inline `mod foo { ... }` blocks. By default
    /// inline mod bodies are reordered with the same rules. Test mods
    /// (`#[cfg(test)]`), `#[macro_use]` mods, and pure-`use` mods
    /// (prelude / __private / sealed re-export shims) are always
    /// skipped because their listing order is part of the contract.
    #[arg(long)]
    no_reorder_inline_mods: bool,

    /// Disable ordering shorter trait paths first. By default,
    /// `impl Debug for Foo` precedes `impl std::fmt::Debug for Foo`
    /// when both target the same type and classify identically.
    #[arg(long)]
    no_short_trait_path_first: bool,

    /// Skip files whose paths contain any of these substrings.
    #[arg(long)]
    exclude: Vec<String>,

    /// Verbose: log every file rewritten (default mode is silent).
    #[arg(short, long)]
    verbose: bool,

    /// Coloured diff and error output: `auto` (default — colour when
    /// stderr is a tty and `NO_COLOR` is unset), `always`, or `never`.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto, value_name = "WHEN")]
    color: ColorChoice,
}

impl Cli {
    fn config(&self) -> Config {
        Config {
            no_mod_before_use: self.no_mod_before_use,
            no_preserve_mod_order: self.no_preserve_mod_order,
            no_trait_before_struct: self.no_trait_before_struct,
            no_import_groups: self.no_import_groups,
            no_impl_grouping: self.no_impl_grouping,
            no_tests_last: self.no_tests_last,
            no_reorder_inline_mods: self.no_reorder_inline_mods,
            no_short_trait_path_first: self.no_short_trait_path_first,
        }
    }
}

impl From<ColorChoice> for DiagColor {
    fn from(c: ColorChoice) -> Self {
        match c {
            ColorChoice::Auto => DiagColor::Auto,
            ColorChoice::Always => DiagColor::Always,
            ColorChoice::Never => DiagColor::Never,
        }
    }
}

fn main() -> ExitCode {
    // Allow invocation as `cargo reorder` (cargo passes "reorder" as argv[1]).
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.get(1).and_then(|s| s.to_str()) == Some("reorder") {
        args.remove(1);
    }
    let cli = Cli::parse_from(args);

    match run(cli) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<i32> {
    let cfg = cli.config();
    let color_enabled = DiagColor::from(cli.color).enabled_for_stderr();

    // Filter mode (stdin→stdout) only when no PATHS and no discovery flag.
    let discovery_flag_given = cli.all || !cli.package.is_empty() || cli.manifest_path.is_some();

    // Explicitly-named files always pass through; directory-walked files
    // are filtered against the cargo-metadata build tree below.
    let mut explicit_files: Vec<PathBuf> = Vec::new();
    let mut walked_files: Vec<PathBuf> = Vec::new();

    if cli.paths.is_empty() {
        if !discovery_flag_given && !io::stdin().is_terminal() {
            // Filter mode: stdin -> stdout. Parse error here is fatal.
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .context("reading stdin")?;
            match reorder_source_with(&buf, &cfg) {
                Ok(out) => {
                    io::stdout().write_all(out.as_bytes())?;
                    return Ok(0);
                }
                Err(ReorderError::Parse(e)) => {
                    let rendered =
                        format_parse_error_colored(Path::new("<stdin>"), &buf, &e, color_enabled);
                    eprint!("{rendered}");
                    return Ok(2);
                }
            }
        }
        let opts = DiscoverOptions {
            all_packages: cli.all,
            packages: &cli.package,
            manifest_path: cli.manifest_path.as_deref(),
        };
        explicit_files = discover(opts).context("discovering files via `cargo metadata`")?;
    } else {
        for path in &cli.paths {
            if path.is_dir() {
                for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                    let p = entry.path();
                    if p.is_file() && p.extension().is_some_and(|e| e == "rs") {
                        walked_files.push(p.to_path_buf());
                    }
                }
            } else {
                explicit_files.push(path.clone());
            }
        }
    }

    // Drop walked files that aren't part of the cargo build tree.
    if !walked_files.is_empty() {
        let opts = DiscoverOptions {
            all_packages: true,
            packages: &[],
            manifest_path: cli.manifest_path.as_deref(),
        };
        if let Ok(tree_paths) = cargo_reorder::discover::discover(opts) {
            if !tree_paths.is_empty() {
                let tree: std::collections::HashSet<PathBuf> = tree_paths.into_iter().collect();
                walked_files
                    .retain(|p| fs::canonicalize(p).ok().is_some_and(|c| tree.contains(&c)));
            }
            // Outside a cargo project: keep every walked file.
        }
    }

    let files: Vec<PathBuf> = explicit_files.into_iter().chain(walked_files).collect();

    let files: Vec<PathBuf> = if cli.exclude.is_empty() {
        files
    } else {
        files
            .into_iter()
            .filter(|p| {
                let s = p.to_string_lossy();
                !cli.exclude.iter().any(|ex| s.contains(ex))
            })
            .collect()
    };

    // Files that reached this point are all build participants (rustfmt
    // parity): parse failures are fatal.
    let mut changed_count = 0usize;
    let mut parse_errors = 0usize;
    let mut hard_errors = 0usize;
    for f in &files {
        match process_file(f, &cfg, cli.check, cli.stdout, cli.verbose, color_enabled) {
            Ok(Outcome::Changed) => changed_count += 1,
            Ok(Outcome::Unchanged) => {}
            Ok(Outcome::ParseError) => parse_errors += 1,
            Err(e) => {
                eprintln!("error: {}: {e:#}", f.display());
                hard_errors += 1;
            }
        }
    }

    if hard_errors > 0 || parse_errors > 0 {
        return Ok(2);
    }
    if cli.check && changed_count > 0 {
        return Ok(1);
    }
    Ok(0)
}

fn process_file(
    path: &Path,
    cfg: &Config,
    check: bool,
    to_stdout: bool,
    verbose: bool,
    color: bool,
) -> Result<Outcome> {
    let original =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let reordered = match reorder_source_with_path(&original, Some(path), cfg) {
        Ok(s) => s,
        Err(ReorderError::Parse(e)) => {
            let rendered = format_parse_error_colored(path, &original, &e, color);
            eprint!("{rendered}");
            return Ok(Outcome::ParseError);
        }
    };
    if reordered == original {
        if to_stdout {
            io::stdout().write_all(reordered.as_bytes())?;
        }
        return Ok(Outcome::Unchanged);
    }
    if to_stdout {
        io::stdout().write_all(reordered.as_bytes())?;
        return Ok(Outcome::Changed);
    }
    if check {
        let mut stderr = io::stderr().lock();
        write_diff_colored(&mut stderr, path, &original, &reordered, color)?;
        return Ok(Outcome::Changed);
    }
    fs::write(path, &reordered).with_context(|| format!("writing {}", path.display()))?;
    if verbose {
        eprintln!("reordered: {}", path.display());
    }
    Ok(Outcome::Changed)
}
