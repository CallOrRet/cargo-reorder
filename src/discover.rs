//! Discover source files the way `cargo fmt` does:
//!
//! 1. Run `cargo metadata --no-deps --format-version 1` to enumerate targets.
//! 2. From every target's `src_path` (typically `src/lib.rs`, `src/main.rs`,
//!    `src/bin/*.rs`, `tests/*.rs`, `examples/*.rs`, `benches/*.rs`,
//!    `build.rs`), walk the module tree by following `mod foo;` declarations.
//! 3. Collect every file actually reached.
//!
//! Files that are not part of any target's module tree (e.g. orphan `.rs`
//! files, build artefacts under `target/`, vendored third-party code) are
//! never discovered and therefore never touched.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use syn::Item;

#[derive(Debug, Default, Clone, Copy)]
pub struct DiscoverOptions<'a> {
    /// Limit to packages with these names (matches `cargo fmt -p NAME`).
    /// Empty slice means "no filter".
    pub packages: &'a [String],

    /// Include every workspace member's targets. Without this, only the
    /// current package (whose Cargo.toml is closest to the cwd) is used.
    /// Mutually exclusive with `packages` — if both are set, `packages` wins.
    pub all_packages: bool,

    /// Override the manifest path (forwarded to `cargo metadata`).
    pub manifest_path: Option<&'a Path>,
}

/// Run `cargo metadata` and walk the module tree. Returns canonical paths.
pub fn discover(opts: DiscoverOptions<'_>) -> Result<Vec<PathBuf>> {
    let entries = cargo_target_entries(opts)?;
    let mut found: BTreeSet<PathBuf> = BTreeSet::new();
    for entry in entries {
        walk_mod_tree(&entry, &mut found);
    }
    Ok(found.into_iter().collect())
}

fn walk_items(items: &[Item], mod_dir: &Path, found: &mut BTreeSet<PathBuf>) {
    for item in items {
        let Item::Mod(m) = item else { continue };
        match &m.content {
            None => {
                if let Some(candidate) = external_mod_file(mod_dir, m) {
                    walk_mod_tree(&candidate, found);
                }
            }
            Some((_, sub_items)) => {
                walk_items(sub_items, &inline_mod_dir(mod_dir, m), found);
            }
        }
    }
}

/// Recursively follow `mod foo;` declarations starting at `entry`.
/// No depth cap — file discovery has to be exhaustive or we'd
/// silently skip reordering deep modules. Canonical-path de-dup
/// already prevents true cycles.
fn walk_mod_tree(entry: &Path, found: &mut BTreeSet<PathBuf>) {
    let canonical = match fs::canonicalize(entry) {
        Ok(p) => p,
        Err(_) => return,
    };
    if !found.insert(canonical.clone()) {
        return;
    }
    let src = match fs::read_to_string(&canonical) {
        Ok(s) => s,
        Err(_) => return,
    };
    let parsed = match syn::parse_file(&src) {
        Ok(p) => p,
        Err(_) => return,
    };
    let mod_dir = mod_directory(&canonical);
    walk_items(&parsed.items, &mod_dir, found);
}

/// rustc's "mod directory" rule: for `lib.rs` / `main.rs` / `mod.rs` /
/// `build.rs`, submodule files live in the same directory; for any other
/// `name.rs`, they live in a sibling `name/` directory.
fn mod_directory(file: &Path) -> PathBuf {
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let parent = file.parent().unwrap_or_else(|| Path::new(""));
    if matches!(stem, "lib" | "main" | "mod" | "build") {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    }
}

/// Synthesise the mod-directory for an inline `mod foo { ... }` body.
/// `<parent>/<name>/`, or `<parent>/<dir(rel)>` when `#[path = "rel"]`
/// is set on the inline mod.
fn inline_mod_dir(parent_dir: &Path, m: &syn::ItemMod) -> PathBuf {
    if let Some(rel) = path_attribute(&m.attrs) {
        return PathBuf::from(&rel)
            .parent()
            .map(|d| parent_dir.join(d))
            .unwrap_or_else(|| parent_dir.to_path_buf());
    }
    parent_dir.join(m.ident.to_string())
}

fn path_attribute(attrs: &[syn::Attribute]) -> Option<String> {
    for a in attrs {
        if !a.path().is_ident("path") {
            continue;
        }
        if let syn::Meta::NameValue(nv) = &a.meta {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
            {
                return Some(s.value());
            }
        }
    }
    None
}
/// Resolve an external `mod foo;` declaration to its `.rs` file.
/// Returns `None` when neither candidate exists on disk.
///
/// Honours `#[path = "..."]`. Without it, tries `<mod_dir>/<name>.rs`
/// then `<mod_dir>/<name>/mod.rs` — rustc's two conventional forms.
fn external_mod_file(mod_dir: &Path, m: &syn::ItemMod) -> Option<PathBuf> {
    if let Some(rel) = path_attribute(&m.attrs) {
        let candidate = mod_dir.join(rel);
        return candidate.is_file().then_some(candidate);
    }
    let name = m.ident.to_string();
    let flat = mod_dir.join(format!("{name}.rs"));
    if flat.is_file() {
        return Some(flat);
    }
    let dir_form = mod_dir.join(&name).join("mod.rs");
    if dir_form.is_file() {
        return Some(dir_form);
    }
    None
}

fn cargo_target_entries(opts: DiscoverOptions<'_>) -> Result<Vec<PathBuf>> {
    let mut cmd = Command::new("cargo");
    cmd.args(["metadata", "--no-deps", "--format-version", "1"]);
    if let Some(p) = opts.manifest_path {
        cmd.arg("--manifest-path").arg(p);
    }
    let out = cmd.output().context(
        "failed to run `cargo metadata`. Is cargo installed and is the cwd inside a cargo project?",
    )?;
    if !out.status.success() {
        return Err(anyhow!(
            "`cargo metadata` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parsing cargo metadata JSON")?;

    // Determine which packages to use.
    let root_id = v
        .get("resolve")
        .and_then(|r| r.get("root"))
        .and_then(|s| s.as_str())
        .map(str::to_owned);

    let packages = v
        .get("packages")
        .and_then(|p| p.as_array())
        .ok_or_else(|| anyhow!("metadata: missing `packages`"))?;

    let mut entries: Vec<PathBuf> = Vec::new();
    let mut matched_names: Vec<String> = Vec::new();
    for pkg in packages {
        let pkg_id = pkg.get("id").and_then(|s| s.as_str()).unwrap_or("");
        let pkg_name = pkg
            .get("name")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_owned();

        // Selection logic, mirroring `cargo fmt`:
        //   * `--package NAME` (one or more): keep packages whose name matches
        //   * `--all`: every workspace member
        //   * neither: only the root package (or all of a virtual workspace)
        let selected = if !opts.packages.is_empty() {
            opts.packages.iter().any(|n| n == &pkg_name)
        } else if opts.all_packages {
            true
        } else if let Some(rid) = &root_id {
            pkg_id == rid
        } else {
            true
        };
        if !selected {
            continue;
        }
        if !opts.packages.is_empty() {
            matched_names.push(pkg_name);
        }
        let targets = match pkg.get("targets").and_then(|t| t.as_array()) {
            Some(t) => t,
            None => continue,
        };
        for tgt in targets {
            if let Some(p) = tgt.get("src_path").and_then(|s| s.as_str()) {
                entries.push(PathBuf::from(p));
            }
        }
    }

    // Surface unknown package names — `cargo fmt -p does-not-exist` errors out,
    // we should too rather than silently doing nothing.
    if !opts.packages.is_empty() {
        for requested in opts.packages {
            if !matched_names.iter().any(|n| n == requested) {
                return Err(anyhow!("package `{requested}` not found in workspace"));
            }
        }
    }
    Ok(entries)
}

