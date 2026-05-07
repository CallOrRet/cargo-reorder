//! CLI entry point. Argument shape is intentionally aligned with
//! `cargo fmt` (`-v`, `-p NAME`, `--all`, `--check`,
//! `--manifest-path PATH`); whatever files cargo fmt would format with a
//! given combination, cargo-reorder operates on the same set. The
//! `--fmt` flag delegates to `cargo fmt` directly so the alignment
//! holds even when both passes run.

use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};

use cargo_reorder::ReorderError;
use cargo_reorder::diagnostic::{
    Color as DiagColor, format_parse_error_colored, write_diff_colored,
};
use cargo_reorder::discover::{DiscoverOptions, discover};
use cargo_reorder::{Config, reorder_source_with, reorder_source_with_path};

enum Outcome {
    Changed,

    Unchanged,

    ParseError,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum ColorChoice {
    Auto,

    Never,

    Always,
}

#[derive(Parser, Debug)]
#[command(
    name = "cargo-reorder",
    about = "Reorder top-level items in Rust source files (use/mod/const/struct/...).",
    version,
    after_help = "\
FILE DISCOVERY:
  cargo-reorder runs `cargo metadata --no-deps` and walks every
  selected package's module tree from each target's root, exactly the
  way `cargo fmt` does. Same arguments → same files. Selection flags
  (`-p` / `--all` / `--manifest-path`) match cargo fmt one-to-one.

  With a piped stdin and no selection flag, source is read from stdin
  and the reordered result is written to stdout (filter mode).

OUTPUT:
  Default mode is silent on success; pass `-v` to log every rewrite.
  `--check` prints a unified diff per file in rustfmt format and exits 1
  if anything would change. Parse errors are rendered rustc-style.

EXAMPLES:
  cargo-reorder                     reorder current package's source tree
  cargo-reorder --all               reorder every workspace member
  cargo-reorder -p foo -p bar       reorder only the named packages
  cargo-reorder --check             CI mode: diff + exit 1 if changes
  cargo-reorder --fmt               run `cargo fmt` first, then reorder
  cargo-reorder --fmt -- --edition 2021
                                    forward rustfmt args through cargo fmt
  cargo-reorder --no-mod-before-use put use declarations before mod
  cargo-reorder < a.rs > b.rs       filter mode (piped stdin)
"
)]
struct Cli {
    /// Process only the named package(s) (matches `cargo fmt -p NAME`).
    #[arg(short = 'p', long = "package", value_name = "NAME")]
    package: Vec<String>,

    /// Process every workspace member (matches `cargo fmt --all`).
    #[arg(long, conflicts_with = "package")]
    all: bool,

    /// Path to the `Cargo.toml` to use for metadata discovery.
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<PathBuf>,

    /// Verbose: log every file rewritten (default mode is silent).
    #[arg(short, long)]
    verbose: bool,

    /// Run `cargo fmt` (with the same `-p` / `--all` / `--manifest-path`
    /// args you passed here) *before* the reorder pass. Skipped under
    /// `--check` so a CI gate doesn't accidentally write to disk;
    /// combine your own `cargo fmt --check` step with `cargo-reorder
    /// --check` if you want both to gate independently.
    #[arg(long)]
    fmt: bool,

    /// Print a unified diff for every file that would change and exit 1
    /// if any do. Does not modify files.
    #[arg(long)]
    check: bool,

    /// Coloured diff and error output: `auto` (default — colour when
    /// stderr is a tty and `NO_COLOR` is unset), `always`, or `never`.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto, value_name = "WHEN")]
    color: ColorChoice,

    /// Extra arguments after `--` are forwarded verbatim to `cargo fmt`
    /// (which in turn forwards them to `rustfmt`). Only meaningful with
    /// `--fmt`. Mirrors cargo fmt's own `cargo fmt -- <rustfmt_options>`.
    #[arg(last = true, value_name = "RUSTFMT_OPTIONS")]
    rustfmt_options: Vec<String>,

    /// Reorder function parameters. Off by default because parameter
    /// order is part of the call contract. When on, the first receiver
    /// (`self`, `mut self`, `&self`, `&mut self`) stays first and the
    /// remaining ordinary identifier parameters use the field grouping
    /// rule.
    #[arg(long)]
    fn_args: bool,
    /// Disable reordering named fields inside `struct` / `union` /
    /// `enum`. By default, fields are grouped by their first word
    /// (snake_case `_` separator or PascalCase / camelCase boundary);
    /// within each group sorted shortest-first; groups emitted in
    /// ascending order of the group's mean name length. ABI- and
    /// semantics-affecting shapes are always skipped (see README).
    #[arg(long)]
    no_fields: bool,
    /// Disable the prefix-group + length sort applied inside `impl`
    /// and `trait` bodies (covers `const` / `type` / `fn` / `async fn`
    /// members). With this flag, every `impl` / `trait` body stays in
    /// source order — useful when methods follow a deliberate sequence
    /// (builder chain, lifecycle order, etc.). Field-level and
    /// top-level grouping stay under `--no-fields`.
    #[arg(long)]
    no_impl_fns: bool,
    /// Do not force `#[cfg(test)] mod ...` to the end of the file.
    #[arg(long)]
    no_tests_last: bool,
    /// Skip recursing into inline `mod foo { ... }` blocks. By default
    /// inline mod bodies are reordered with the same rules. Test mods
    /// (`#[cfg(test)]`), `#[macro_use]` mods, and pure-`use` mods
    /// (prelude / __private / sealed re-export shims) are always
    /// skipped because their listing order is part of the contract.
    #[arg(long)]
    no_inline_mods: bool,
    /// Disable anchoring `impl` blocks to their target type.
    #[arg(long)]
    no_impl_grouping: bool,
    /// Disable splitting `use` items into std / external / crate groups.
    #[arg(long)]
    no_import_groups: bool,
    /// Disable putting `mod foo;` before `use ...;` (i.e., switch to
    /// use-first). Default is mod-first because the pair-majority
    /// sample of 21 real-world crates leans 12-vs-7 toward mod-first
    /// (see README). Pass this flag if your project is on the
    /// use-first side (notably `regex`, `ripgrep`, `cargo`, `chrono`,
    /// `tracing`).
    #[arg(long)]
    no_mod_before_use: bool,
    /// Disable ordering shorter trait paths first. By default,
    /// `impl Debug for Foo` precedes `impl std::fmt::Debug for Foo`
    /// when both target the same type and classify identically.
    #[arg(long)]
    no_short_trait_first: bool,
    /// Preserve existing blank lines between reordered multi-line
    /// field-like entries. By default those blank lines are trimmed.
    /// Blank lines before the first emitted field are always trimmed.
    #[arg(long)]
    no_trim_field_blanks: bool,
    /// Disable preserving `pub mod` / `mod` source order. With this
    /// on, `pub mod` items are sorted before private `mod` items with
    /// a blank line between the two groups. Default is to preserve
    /// source order — empirically interleaved is the modal pattern
    /// (11/19 projects in the README sample).
    #[arg(long)]
    no_preserve_mod_order: bool,
    /// Disable reordering single-line field-like lists: `struct S { b:
    /// u8, a: u8 }` and `S { b: 1, a: 2 }`. By default those lists are
    /// permuted in place and stay on one line.
    #[arg(long)]
    no_single_line_fields: bool,
    /// Disable putting `trait` ahead of `enum` / `struct` / `union`
    /// (i.e., switch to struct-first). Default is trait-first — in
    /// the 21-project sample 14/20 lean trait-first under
    /// pair-majority. Pass this flag for projects that consistently
    /// put structs / enums first.
    #[arg(long)]
    no_trait_before_struct: bool,
}

impl Cli {
    fn config(&self) -> Config {
        Config {
            fn_args: self.fn_args,
            no_fields: self.no_fields,
            no_impl_fns: self.no_impl_fns,
            no_tests_last: self.no_tests_last,
            no_inline_mods: self.no_inline_mods,
            no_impl_grouping: self.no_impl_grouping,
            no_import_groups: self.no_import_groups,
            no_mod_before_use: self.no_mod_before_use,
            no_short_trait_first: self.no_short_trait_first,
            no_trim_field_blanks: self.no_trim_field_blanks,
            no_preserve_mod_order: self.no_preserve_mod_order,
            no_single_line_fields: self.no_single_line_fields,
            no_trait_before_struct: self.no_trait_before_struct,
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

    // Filter mode: no selection flag + piped stdin → read source from
    // stdin, write reordered result to stdout. Skips `--fmt` (no file
    // path to hand cargo fmt) and ignores `-v` / `--check` (filter
    // mode is for one-shot pipelines).
    let no_selection = !cli.all && cli.package.is_empty() && cli.manifest_path.is_none();
    if no_selection && !io::stdin().is_terminal() {
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

    if cli.fmt && !cli.check {
        run_cargo_fmt(&cli).context("running `cargo fmt` before reorder")?;
    }

    let opts = DiscoverOptions {
        all_packages: cli.all,
        packages: &cli.package,
        manifest_path: cli.manifest_path.as_deref(),
    };
    let files = discover(opts).context("discovering files via `cargo metadata`")?;

    let mut changed_count = 0usize;
    let mut parse_errors = 0usize;
    let mut hard_errors = 0usize;
    for f in &files {
        match process_file(f, &cfg, cli.check, cli.verbose, color_enabled) {
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

/// Forward the user's `-p` / `--all` / `--manifest-path` to
/// `cargo fmt`. Same arguments → same files cargo fmt would format.
/// cargo fmt invokes `rustfmt` per package internally, so we don't
/// need to worry about ARG_MAX / chunking / edition handling — that's
/// all on cargo fmt's side.
fn run_cargo_fmt(cli: &Cli) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("fmt");
    if cli.all {
        cmd.arg("--all");
    }
    for pkg in &cli.package {
        cmd.arg("-p").arg(pkg);
    }
    if let Some(mp) = &cli.manifest_path {
        cmd.arg("--manifest-path").arg(mp);
    }
    if !cli.rustfmt_options.is_empty() {
        cmd.arg("--");
        cmd.args(&cli.rustfmt_options);
    }
    let status = cmd
        .status()
        .context("failed to spawn `cargo fmt` (is cargo installed?)")?;
    if !status.success() {
        anyhow::bail!("`cargo fmt` exited with status {status}");
    }
    Ok(())
}

fn process_file(
    path: &Path,
    cfg: &Config,
    check: bool,
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
        return Ok(Outcome::Unchanged);
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
