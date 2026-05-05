//! Empirical sampling tool for the README's "On --foo" sections.
//!
//! Walks each given directory, parses every `.rs` file with `syn`, and
//! collects three observations *per scope*:
//!
//! 1. mod-vs-use — does the first `mod foo;` appear before the first
//!    `use ...;`? (Drives `--mod-before-use` stats.)
//! 2. pub/priv mod — pattern of `pub mod` vs private `mod` declarations
//!    within the scope: P+M+ / M+P+ / interleaved.
//!    (Drives `--pub-mod-first` stats.)
//! 3. trait-vs-struct — does the first `trait` appear before the first
//!    `struct` / `enum` / `union`? (Drives `--struct-before-trait` stats.)
//!
//! A "scope" is the file's top level OR the body of an inline
//! `mod foo { ... }`. Inline mods become their own observations so an
//! inner module's organisational style counts independently of its
//! parent. **Test mods** — anything named `tests` / `test` or carrying a
//! `#[cfg(test)]` attribute — are skipped entirely (no observation
//! contributed for the mod itself, and no recursion into its body).
//!
//! Because we use `syn`, all visibility variants (`pub`, `pub(crate)`,
//! `pub(super)`, `pub(in ::path)`) and inline-mod nesting are handled
//! uniformly — no ad-hoc regex pitfalls.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use syn::Item;

#[derive(Debug, Default, Clone, Copy)]
struct Counts {
    mod_first: u32,
    use_first: u32,
    pub_first: u32,
    priv_first: u32,
    interleaved: u32,
    trait_first: u32,
    struct_first: u32,
}

impl std::ops::AddAssign for Counts {
    fn add_assign(&mut self, r: Self) {
        self.mod_first += r.mod_first;
        self.use_first += r.use_first;
        self.pub_first += r.pub_first;
        self.priv_first += r.priv_first;
        self.interleaved += r.interleaved;
        self.trait_first += r.trait_first;
        self.struct_first += r.struct_first;
    }
}

struct FileObs {
    first_mod: Option<usize>,
    first_use: Option<usize>,
    first_trait: Option<usize>,
    first_type: Option<usize>,
    mod_seq: String,
}

fn is_test_mod(m: &syn::ItemMod) -> bool {
    let name = m.ident.to_string();
    if name == "tests" || name == "test" {
        return true;
    }
    m.attrs.iter().any(|a| {
        if !a.path().is_ident("cfg") {
            return false;
        }
        let mut found = false;
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("test") {
                found = true;
            }
            Ok(())
        });
        found
    })
}

/// Observe one scope (a Vec<Item> belonging to either a File or an
/// inline `mod foo { ... }`). Test mods are skipped — they neither
/// contribute their own slot to the observation nor get recursed into.
fn observe(items: &[Item]) -> FileObs {
    let mut o = FileObs {
        first_mod: None,
        first_use: None,
        first_trait: None,
        first_type: None,
        mod_seq: String::new(),
    };
    for (idx, item) in items.iter().enumerate() {
        match item {
            Item::Mod(m) => {
                if is_test_mod(m) {
                    continue; // skip tests mods entirely
                }
                if o.first_mod.is_none() {
                    o.first_mod = Some(idx);
                }
                let is_pub = !matches!(m.vis, syn::Visibility::Inherited);
                o.mod_seq.push(if is_pub { 'P' } else { 'M' });
            }
            Item::Use(_) => {
                o.first_use.get_or_insert(idx);
            }
            Item::Trait(_) | Item::TraitAlias(_) => {
                o.first_trait.get_or_insert(idx);
            }
            Item::Struct(_) | Item::Enum(_) | Item::Union(_) => {
                o.first_type.get_or_insert(idx);
            }
            _ => {}
        }
    }
    o
}

/// Recurse: count the current scope, then walk every inline (non-test)
/// `mod` and treat its body as a fresh scope.
fn sample_scope(items: &[Item], counts: &mut Counts, scopes: &mut u32) {
    *counts += classify(&observe(items));
    *scopes += 1;
    for item in items {
        if let Item::Mod(m) = item {
            if is_test_mod(m) {
                continue;
            }
            if let Some((_, inner)) = &m.content {
                sample_scope(inner, counts, scopes);
            }
        }
    }
}

fn classify(o: &FileObs) -> Counts {
    let mut c = Counts::default();
    if let (Some(m), Some(u)) = (o.first_mod, o.first_use) {
        if m < u {
            c.mod_first = 1;
        } else {
            c.use_first = 1;
        }
    }
    if o.mod_seq.contains('P') && o.mod_seq.contains('M') {
        let has_mp_transition = o.mod_seq.contains("MP");
        let has_pm_transition = o.mod_seq.contains("PM");
        if !has_mp_transition {
            c.pub_first = 1; // all P then all M
        } else if !has_pm_transition {
            c.priv_first = 1; // all M then all P
        } else {
            c.interleaved = 1;
        }
    }
    if let (Some(t), Some(s)) = (o.first_trait, o.first_type) {
        if t < s {
            c.trait_first = 1;
        } else {
            c.struct_first = 1;
        }
    }
    c
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            // Skip build artifacts and version-control folders.
            if name == "target" || name == ".git" {
                continue;
            }
            walk_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn sample_project(name: &str, dir: &Path) -> Counts {
    let mut files = Vec::new();
    walk_rs(dir, &mut files);
    let mut total = Counts::default();
    let mut parse_skipped = 0u32;
    let mut scopes = 0u32;
    for f in &files {
        let src = match fs::read_to_string(f) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let parsed = match syn::parse_file(&src) {
            Ok(p) => p,
            Err(_) => {
                parse_skipped += 1;
                continue;
            }
        };
        sample_scope(&parsed.items, &mut total, &mut scopes);
    }
    let mu_total = total.mod_first + total.use_first;
    let pp_total = total.pub_first + total.priv_first + total.interleaved;
    let ts_total = total.trait_first + total.struct_first;
    eprintln!(
        "  {:<14} files={:5} scopes={:5} parse-skipped={:3}  \
         mod/use={}+{}={}  pub/priv/inter={}+{}+{}={}  \
         trait/struct={}+{}={}",
        name,
        files.len(),
        scopes,
        parse_skipped,
        total.mod_first,
        total.use_first,
        mu_total,
        total.pub_first,
        total.priv_first,
        total.interleaved,
        pp_total,
        total.trait_first,
        total.struct_first,
        ts_total,
    );
    total
}

fn main() {
    let args: Vec<PathBuf> = env::args_os().skip(1).map(PathBuf::from).collect();
    if args.is_empty() {
        eprintln!("usage: sample-stats <project-dir>...");
        std::process::exit(2);
    }

    let mut per_project: BTreeMap<String, Counts> = BTreeMap::new();
    eprintln!("Per-project (parse-skipped: files where syn::parse_file errored):");
    for dir in &args {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.display().to_string());
        let counts = sample_project(&name, dir);
        per_project.insert(name, counts);
    }

    let mut totals = Counts::default();
    for c in per_project.values() {
        totals += *c;
    }

    println!("\n## mod-vs-use (which appears first at the top level)\n");
    println!("| project | use-first | mod-first |");
    println!("| --- | --: | --: |");
    for (name, c) in &per_project {
        let t = c.mod_first + c.use_first;
        if t == 0 {
            continue;
        }
        let pct = |x: u32| format!("{}%", (x as f64 * 100.0 / t as f64).round() as i64);
        println!("| {} | {} | {} |", name, pct(c.use_first), pct(c.mod_first));
    }
    let mu = totals.mod_first + totals.use_first;
    println!(
        "\n**Aggregate ({} files)**: use-first {:.0}%, mod-first {:.0}%",
        mu,
        100.0 * totals.use_first as f64 / mu as f64,
        100.0 * totals.mod_first as f64 / mu as f64,
    );

    println!("\n## pub-mod / priv-mod arrangement\n");
    println!("| project | pub-first | priv-first | interleaved |");
    println!("| --- | --: | --: | --: |");
    for (name, c) in &per_project {
        let t = c.pub_first + c.priv_first + c.interleaved;
        if t == 0 {
            continue;
        }
        println!(
            "| {} | {} | {} | {} |",
            name, c.pub_first, c.priv_first, c.interleaved
        );
    }
    let pp = totals.pub_first + totals.priv_first + totals.interleaved;
    println!(
        "\n**Aggregate ({} files)**: pub-first {:.0}%, priv-first {:.0}%, interleaved {:.0}%",
        pp,
        100.0 * totals.pub_first as f64 / pp as f64,
        100.0 * totals.priv_first as f64 / pp as f64,
        100.0 * totals.interleaved as f64 / pp as f64,
    );

    println!("\n## trait-vs-(struct/enum/union) (which appears first)\n");
    println!("| project | trait-first | struct-first |");
    println!("| --- | --: | --: |");
    for (name, c) in &per_project {
        let t = c.trait_first + c.struct_first;
        if t == 0 {
            continue;
        }
        println!("| {} | {} | {} |", name, c.trait_first, c.struct_first);
    }
    let ts = totals.trait_first + totals.struct_first;
    println!(
        "\n**Aggregate ({} files)**: trait-first {:.0}%, struct-first {:.0}%",
        ts,
        100.0 * totals.trait_first as f64 / ts as f64,
        100.0 * totals.struct_first as f64 / ts as f64,
    );
}
