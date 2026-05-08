//! Empirical sampling tool for the README's "On --foo" sections.
//!
//! Walks each given directory, parses every `.rs` file with `syn`, and
//! collects three observations *per scope* using **pair-majority**
//! semantics: for two item kinds A and B, count every (a, b) pair and
//! see whether more of them have a-before-b or b-before-a. The scope
//! votes for the side that wins more pairs; an exact tie is its own
//! bucket.
//!
//! 1. mod-vs-use — among (`mod foo;`, `use ...;`) pairs, does the
//!    majority have mod before use? (Drives `--no-mod-before-use` stats.)
//! 2. pub/priv mod — among (`pub mod foo;`, `mod foo;`) pairs, does
//!    the majority have `pub mod` before private `mod`? (Drives
//!    `--no-pub-mod-first` stats.)
//! 3. trait-vs-struct — among (`trait`, `struct`/`enum`/`union`) pairs,
//!    does the majority have trait before the type? (Drives
//!    `--no-trait-before-struct` stats.)
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

/// Per-category counters: each scope contributes exactly 1 to one of
/// the three buckets (the third is "tied" when pair counts are equal).
#[derive(Debug, Default, Clone, Copy)]
struct Counts {
    mu_tied: u32,
    pp_tied: u32,
    ts_tied: u32,
    mod_first: u32,
    use_first: u32,
    pub_first: u32,
    priv_first: u32,
    trait_first: u32,
    struct_first: u32,
}

impl std::ops::AddAssign for Counts {
    fn add_assign(&mut self, r: Self) {
        self.mod_first += r.mod_first;
        self.use_first += r.use_first;
        self.mu_tied += r.mu_tied;
        self.pub_first += r.pub_first;
        self.priv_first += r.priv_first;
        self.pp_tied += r.pp_tied;
        self.trait_first += r.trait_first;
        self.struct_first += r.struct_first;
        self.ts_tied += r.ts_tied;
    }
}

/// All item positions we need to do pair counting on. Indices are
/// each item's position within the scope's `items` Vec; the absolute
/// value doesn't matter, only relative ordering does.
#[derive(Default)]
struct FileObs {
    mods: Vec<usize>, // private + pub external mods
    uses: Vec<usize>,
    types: Vec<usize>, // struct / enum / union
    traits: Vec<usize>,
    pub_mods: Vec<usize>,  // external `pub mod` only
    priv_mods: Vec<usize>, // external private `mod` only
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

    // Aggregate by *project*, not by file: each project that has any
    // observation in a category votes once for whichever side wins its
    // internal majority (or "tied" on an exact split). This avoids
    // letting large projects (cargo, tokio) dominate the rollup.
    println!("\n## mod-vs-use (pair-majority within each scope)\n");
    println!("| project | use-first | mod-first | tied |");
    println!("| --- | --: | --: | --: |");
    let (mut p_use, mut p_mod, mut p_tied) = (0u32, 0u32, 0u32);
    for (name, c) in &per_project {
        let t = c.mod_first + c.use_first + c.mu_tied;
        if t == 0 {
            continue;
        }
        let winner_mods = c.mod_first > c.use_first && c.mod_first > c.mu_tied;
        let winner_uses = c.use_first > c.mod_first && c.use_first > c.mu_tied;
        if winner_mods {
            p_mod += 1;
        } else if winner_uses {
            p_use += 1;
        } else {
            p_tied += 1;
        }
        println!(
            "| {} | {} | {} | {} |",
            name, c.use_first, c.mod_first, c.mu_tied
        );
    }
    print_project_rollup("use-first", p_use, "mod-first", p_mod, p_tied);

    println!("\n## pub-mod / priv-mod (pair-majority within each scope)\n");
    println!("| project | pub-first | priv-first | tied |");
    println!("| --- | --: | --: | --: |");
    let (mut p_pub, mut p_priv, mut p_pp_tied) = (0u32, 0u32, 0u32);
    for (name, c) in &per_project {
        let t = c.pub_first + c.priv_first + c.pp_tied;
        if t == 0 {
            continue;
        }
        let winner_pub = c.pub_first > c.priv_first && c.pub_first > c.pp_tied;
        let winner_priv = c.priv_first > c.pub_first && c.priv_first > c.pp_tied;
        if winner_pub {
            p_pub += 1;
        } else if winner_priv {
            p_priv += 1;
        } else {
            p_pp_tied += 1;
        }
        println!(
            "| {} | {} | {} | {} |",
            name, c.pub_first, c.priv_first, c.pp_tied
        );
    }
    print_project_rollup("pub-first", p_pub, "priv-first", p_priv, p_pp_tied);

    println!("\n## trait-vs-(struct/enum/union) (pair-majority within each scope)\n");
    println!("| project | trait-first | struct-first | tied |");
    println!("| --- | --: | --: | --: |");
    let (mut p_trait, mut p_struct, mut p_ts_tied) = (0u32, 0u32, 0u32);
    for (name, c) in &per_project {
        let t = c.trait_first + c.struct_first + c.ts_tied;
        if t == 0 {
            continue;
        }
        let winner_trait = c.trait_first > c.struct_first && c.trait_first > c.ts_tied;
        let winner_struct = c.struct_first > c.trait_first && c.struct_first > c.ts_tied;
        if winner_trait {
            p_trait += 1;
        } else if winner_struct {
            p_struct += 1;
        } else {
            p_ts_tied += 1;
        }
        println!(
            "| {} | {} | {} | {} |",
            name, c.trait_first, c.struct_first, c.ts_tied
        );
    }
    print_project_rollup("trait-first", p_trait, "struct-first", p_struct, p_ts_tied);
}

/// Observe one scope (a Vec<Item> belonging to either a File or an
/// inline `mod foo { ... }`). Test mods are skipped — they neither
/// contribute their own slot to the observation nor get recursed into.
fn observe(items: &[Item]) -> FileObs {
    let mut o = FileObs::default();
    for (idx, item) in items.iter().enumerate() {
        match item {
            Item::Mod(m) => {
                if is_test_mod(m) {
                    continue;
                }
                // Only external `mod foo;` declarations count for the
                // mod-vs-use and pub/priv-mod stats. Inline
                // `mod foo { ... }` blocks are a different construct
                // (their bodies are sampled as a fresh scope below).
                if m.content.is_some() {
                    continue;
                }
                o.mods.push(idx);
                if matches!(m.vis, syn::Visibility::Inherited) {
                    o.priv_mods.push(idx);
                } else {
                    o.pub_mods.push(idx);
                }
            }
            Item::Use(_) => o.uses.push(idx),
            Item::Trait(_) | Item::TraitAlias(_) => o.traits.push(idx),
            Item::Struct(_) | Item::Enum(_) | Item::Union(_) => o.types.push(idx),
            _ => {}
        }
    }
    o
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
            // Skip build artifacts, VCS folders, and the conventional
            // non-library directories (`tests/` / `examples/` /
            // `benches/`). The latter follow integration-style
            // conventions — `mod common;` helpers, derive-UI fixtures,
            // throw-away structs — that don't reflect how a project's
            // library source organises items.
            if matches!(
                name.to_str(),
                Some("target" | ".git" | "tests" | "examples" | "benches")
            ) {
                continue;
            }
            walk_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Number of (a, b) pairs with a < b. O(|a|·|b|), fine for our scope sizes.
fn count_lt(a: &[usize], b: &[usize]) -> u64 {
    a.iter()
        .map(|&ai| b.iter().filter(|&&bi| ai < bi).count() as u64)
        .sum()
}

fn classify(o: &FileObs) -> Counts {
    let mut c = Counts::default();

    if !o.mods.is_empty() && !o.uses.is_empty() {
        let m_first = count_lt(&o.mods, &o.uses);
        let u_first = count_lt(&o.uses, &o.mods);
        match m_first.cmp(&u_first) {
            std::cmp::Ordering::Greater => c.mod_first = 1,
            std::cmp::Ordering::Less => c.use_first = 1,
            std::cmp::Ordering::Equal => c.mu_tied = 1,
        }
    }

    if !o.pub_mods.is_empty() && !o.priv_mods.is_empty() {
        let p_first = count_lt(&o.pub_mods, &o.priv_mods);
        let m_first = count_lt(&o.priv_mods, &o.pub_mods);
        match p_first.cmp(&m_first) {
            std::cmp::Ordering::Greater => c.pub_first = 1,
            std::cmp::Ordering::Less => c.priv_first = 1,
            std::cmp::Ordering::Equal => c.pp_tied = 1,
        }
    }

    if !o.traits.is_empty() && !o.types.is_empty() {
        let t_first = count_lt(&o.traits, &o.types);
        let s_first = count_lt(&o.types, &o.traits);
        match t_first.cmp(&s_first) {
            std::cmp::Ordering::Greater => c.trait_first = 1,
            std::cmp::Ordering::Less => c.struct_first = 1,
            std::cmp::Ordering::Equal => c.ts_tied = 1,
        }
    }

    c
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
    let mu_total = total.mod_first + total.use_first + total.mu_tied;
    let pp_total = total.pub_first + total.priv_first + total.pp_tied;
    let ts_total = total.trait_first + total.struct_first + total.ts_tied;
    eprintln!(
        "  {:<14} files={:5} scopes={:5} parse-skipped={:3}  \
         mod/use={}+{}+{}={}  pub/priv={}+{}+{}={}  \
         trait/struct={}+{}+{}={}",
        name,
        files.len(),
        scopes,
        parse_skipped,
        total.mod_first,
        total.use_first,
        total.mu_tied,
        mu_total,
        total.pub_first,
        total.priv_first,
        total.pp_tied,
        pp_total,
        total.trait_first,
        total.struct_first,
        total.ts_tied,
        ts_total,
    );
    total
}

fn print_project_rollup(label_a: &str, a: u32, label_b: &str, b: u32, tied: u32) {
    let total = a + b + tied;
    let pct = |x: u32| 100.0 * x as f64 / total as f64;
    if tied > 0 {
        println!(
            "\n**Aggregate ({} projects)**: {} {} ({:.0}%), {} {} ({:.0}%), tied {} ({:.0}%)",
            total,
            label_a,
            a,
            pct(a),
            label_b,
            b,
            pct(b),
            tied,
            pct(tied)
        );
    } else {
        println!(
            "\n**Aggregate ({} projects)**: {} {} ({:.0}%), {} {} ({:.0}%)",
            total,
            label_a,
            a,
            pct(a),
            label_b,
            b,
            pct(b)
        );
    }
}
