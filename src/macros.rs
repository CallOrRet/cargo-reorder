//! Macro placement pipeline.
//!
//! `macro_rules!` is textually scoped, so moving a definition past one
//! of its use sites breaks compilation. The pipeline runs as a sequence
//! of stage functions over a parsed scope's items:
//!
//!   1. [`compute_segments`] — assign each item to a barrier segment
//!      so bare top-level macro invocations stay pinned in place.
//!   2. [`collect_macro_defs`] — list every `macro_rules!` in source
//!      order. The accompanying `macro_names` set is reused throughout.
//!   3. [`collect_item_uses`] — per item, the set of local macros it
//!      bare-calls (self-references stripped).
//!   4. [`compute_effective_uses`] — transitively expand macro→macro
//!      edges so a non-macro caller of `a!` also "uses" everything
//!      `a!`'s body invokes (collapses mutually-recursive macros into
//!      a single fixpoint without oscillation).
//!   5. [`collect_mod_constraints`] — for each external `mod foo;`,
//!      open the child file (or fall back to "conservative") to learn
//!      which local macros it actually needs textually before the mod.
//!   6. [`yank_macro_defs`] — fixpoint loop: for every `macro_rules!`,
//!      pull it back to before its earliest constraining caller.
//!
//! [`find_calls`] and [`scan_child`] are the lower-level primitives that
//! the higher-level stages call into.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};
use syn::Item;

/// Collect `Ident !` pairs whose ident is in `names`. Recurses into
/// delimited groups so calls inside `impl` blocks, fn bodies, etc.
/// are found.
pub(crate) fn find_calls(ts: TokenStream, names: &HashSet<String>, found: &mut HashSet<String>) {
    let toks: Vec<TokenTree> = ts.into_iter().collect();
    for i in 0..toks.len() {
        if let TokenTree::Ident(id) = &toks[i] {
            if let Some(TokenTree::Punct(p)) = toks.get(i + 1) {
                if p.as_char() == '!' {
                    let n = id.to_string();
                    if names.contains(&n) {
                        found.insert(n);
                    }
                }
            }
        }
        if let TokenTree::Group(g) = &toks[i] {
            find_calls(g.stream(), names, found);
        }
    }
}

/// Names brought into local scope by any `use ... ::name;` whose
/// final ident matches `local_macros`. Globs are conservatively
/// ignored.
pub(crate) fn collect_use_leaves(
    tree: &syn::UseTree,
    local_macros: &HashSet<String>,
    out: &mut HashSet<String>,
) {
    use syn::UseTree::*;
    match tree {
        Path(p) => collect_use_leaves(&p.tree, local_macros, out),
        Name(n) => {
            let s = n.ident.to_string();
            if local_macros.contains(&s) {
                out.insert(s);
            }
        }
        Rename(r) => {
            let s = r.rename.to_string();
            if local_macros.contains(&s) {
                out.insert(s);
            }
        }
        Glob(_) => {}
        Group(g) => {
            for inner in &g.items {
                collect_use_leaves(inner, local_macros, out);
            }
        }
    }
}

/// For one `mod foo;` declaration in a scope whose mod-directory is
/// `mod_dir`, find which of `local_macros` the child file genuinely
/// needs textually before it.
///
/// `mod_dir` follows rustc's "mod directory" rule applied to the
/// scope: for the top-level body of `lib.rs` / `main.rs` / `mod.rs`
/// it's `parent.parent()`; for a regular `name.rs` it's the sibling
/// `name/` directory; for an inline `mod foo { ... }` body it's the
/// enclosing scope's mod-directory joined with `foo` (or with the
/// `#[path]` override's parent dir).
///
/// Returns `Some(set)` when the child opened cleanly (set may be
/// empty — child imports or doesn't use any of our macros), or
/// `None` when the child can't be read or parsed (caller falls back
/// to the conservative "every mod constrains every macro" rule).
pub(crate) fn scan_child(
    mod_dir: &Path,
    m: &syn::ItemMod,
    local_macros: &HashSet<String>,
    item_to_tokens: impl Fn(&Item) -> TokenStream,
) -> Option<HashSet<String>> {
    let candidate = crate::discover::external_mod_file(mod_dir, m)?;
    let src = std::fs::read_to_string(&candidate).ok()?;
    let parsed = syn::parse_file(&src).ok()?;

    let mut imported: HashSet<String> = HashSet::new();
    for it in &parsed.items {
        if let Item::Use(u) = it {
            collect_use_leaves(&u.tree, local_macros, &mut imported);
        }
    }
    let mut bare: HashSet<String> = HashSet::new();
    for it in &parsed.items {
        find_calls(item_to_tokens(it), local_macros, &mut bare);
    }
    Some(bare.difference(&imported).cloned().collect())
}

/// Stage 1: items that *export* macros into the surrounding scope
/// pin themselves on a private odd segment so nothing crosses them,
/// preserving textual visibility order. Two cases:
///
/// * Bare top-level macro invocations like `lazy_static! { ... }`,
///   which expand in place.
/// * `#[macro_use] mod foo;` (external or inline) — `macro_rules!`
///   defined inside `foo` leak into the parent scope starting at the
///   `mod` declaration. Any later sibling that bare-calls one of those
///   macros only compiles if the `mod` declaration stays before it.
///   Without a barrier, `--pub-mod-first` (or any flag that re-buckets
///   mods) can move a sibling above the `#[macro_use] mod` line and
///   break the build (observed on syn's lib.rs).
pub(crate) fn compute_segments(items: &[Item]) -> Vec<u32> {
    let mut barriers_seen = 0u32;
    items
        .iter()
        .map(|item| {
            let is_barrier = match item {
                Item::Macro(m) => m.ident.is_none(),
                Item::Mod(m) => has_macro_use_attr(&m.attrs),
                _ => false,
            };
            if is_barrier {
                let s = barriers_seen * 2 + 1;
                barriers_seen += 1;
                return s;
            }
            barriers_seen * 2
        })
        .collect()
}

fn has_macro_use_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("macro_use"))
}

/// Stage 2: collect every `macro_rules! NAME` in source order. A `Vec`
/// (not a map) because `cfg`-gated alternatives can repeat a name and
/// the yank pass needs deterministic source ordering.
pub(crate) fn collect_macro_defs(items: &[Item]) -> Vec<(String, usize)> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| match item {
            Item::Macro(m) => m.ident.as_ref().map(|n| (n.to_string(), i)),
            _ => None,
        })
        .collect()
}

/// Stage 3: for each item, the set of local macros it bare-invokes
/// (excluding self-references, which don't impose ordering). `item_streams[i]`
/// must be the TokenStream representation of `items[i]`.
pub(crate) fn collect_item_uses(
    items: &[Item],
    item_streams: &[TokenStream],
    macro_names: &HashSet<String>,
) -> HashMap<usize, HashSet<String>> {
    let mut item_uses: HashMap<usize, HashSet<String>> = HashMap::new();
    if macro_names.is_empty() {
        return item_uses;
    }
    for (i, (item, ts)) in items.iter().zip(item_streams).enumerate() {
        let mut found: HashSet<String> = HashSet::new();
        find_calls(ts.clone(), macro_names, &mut found);
        if let Item::Macro(m) = item {
            if let Some(name) = &m.ident {
                found.remove(&name.to_string());
            }
        }
        if !found.is_empty() {
            item_uses.insert(i, found);
        }
    }
    item_uses
}

/// Stage 4: expand macro→macro edges transitively so a non-macro caller
/// of `a!` is treated as using everything `a!`'s body invokes (and
/// their callees, recursively). Mutually-recursive macros collapse
/// into one shared constraint set instead of oscillating the fixpoint.
pub(crate) fn compute_effective_uses(
    item_uses: &HashMap<usize, HashSet<String>>,
    macro_defs: &[(String, usize)],
) -> HashMap<usize, HashSet<String>> {
    let macro_def_orig_set: HashSet<usize> = macro_defs.iter().map(|(_, i)| *i).collect();
    let mut macro_to_macro: HashMap<String, HashSet<String>> = HashMap::new();
    for (name, def_orig) in macro_defs {
        let direct = item_uses.get(def_orig).cloned().unwrap_or_default();
        macro_to_macro.insert(name.clone(), direct);
    }
    let mut effective: HashMap<usize, HashSet<String>> = HashMap::new();
    for (&caller_orig, direct) in item_uses {
        if macro_def_orig_set.contains(&caller_orig) {
            continue;
        }
        let mut all: HashSet<String> = direct.clone();
        let mut stack: Vec<String> = direct.iter().cloned().collect();
        while let Some(m) = stack.pop() {
            if let Some(reached) = macro_to_macro.get(&m) {
                for r in reached {
                    if all.insert(r.clone()) {
                        stack.push(r.clone());
                    }
                }
            }
        }
        effective.insert(caller_orig, all);
    }
    effective
}

/// Stage 5: per external `mod foo;`, decide which local macros that
/// child file genuinely requires textually before the `mod` line.
/// Returns `(precise, conservative)`:
///   * `precise[mod_orig]` = exact set, populated when the child file
///     was found and parsed cleanly (and at least one macro is needed).
///   * `conservative` = mod indices whose child couldn't be opened
///     (no `mod_dir`, missing file, parse error). The yank pass treats
///     every macro as a potential need for these.
pub(crate) fn collect_mod_constraints(
    items: &[Item],
    mod_dir: Option<&Path>,
    macro_names: &HashSet<String>,
    item_to_tokens: impl Fn(&Item) -> TokenStream + Copy,
) -> (HashMap<usize, HashSet<String>>, Vec<usize>) {
    let mut precise: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut conservative: Vec<usize> = Vec::new();
    if macro_names.is_empty() {
        return (precise, conservative);
    }
    for (mod_orig, item) in items.iter().enumerate() {
        let Item::Mod(m) = item else { continue };
        if m.content.is_some() {
            continue;
        }
        match mod_dir {
            Some(dir) => match scan_child(dir, m, macro_names, item_to_tokens) {
                Some(needs) if !needs.is_empty() => {
                    precise.insert(mod_orig, needs);
                }
                Some(_) => {} // child opened, doesn't need any of our macros
                None => conservative.push(mod_orig),
            },
            None => conservative.push(mod_orig),
        }
    }
    (precise, conservative)
}

/// Stage 6: yank each `macro_rules!` back to before its earliest
/// constraining caller in the current ordering. Iterates to fixpoint
/// in source order (deterministic). The `iter_cap` is a belt-and-
/// braces guard against pathological inputs that could otherwise
/// loop; the transitive expansion in stage 4 already eliminates the
/// known oscillating case (mutually-recursive macros).
///
/// `original_index` extracts each block's source-order index so this
/// function stays agnostic about the caller's block representation.
pub(crate) fn yank_macro_defs<T>(
    blocks: &mut Vec<T>,
    macro_defs: &[(String, usize)],
    effective_uses: &HashMap<usize, HashSet<String>>,
    mod_constraints: &HashMap<usize, HashSet<String>>,
    conservative_mods: &[usize],
    original_index: impl Fn(&T) -> usize,
) {
    if macro_defs.is_empty() {
        return;
    }
    // Reverse indexes built once: macro name -> origs that constrain it.
    // The constraint sets don't change as macro defs move within `blocks`,
    // so this hoists the inner O(callers + mods) scan from per-iter to
    // per-name. Algorithmic: O(D · E) -> O(E) over the fixpoint loop.
    let mut callers_of: HashMap<&str, Vec<usize>> = HashMap::new();
    for (&caller_orig, used) in effective_uses {
        for name in used {
            callers_of
                .entry(name.as_str())
                .or_default()
                .push(caller_orig);
        }
    }
    let mut mods_of: HashMap<&str, Vec<usize>> = HashMap::new();
    for (&mod_orig, needs) in mod_constraints {
        for name in needs {
            mods_of.entry(name.as_str()).or_default().push(mod_orig);
        }
    }

    // Dense `pos` lookup: orig values are [0, n) in the standard caller,
    // so a Vec is faster than a HashMap rebuild every iteration. Sentinel
    // `usize::MAX` is harmless under `<` comparisons because `MAX < MAX`
    // is false.
    let max_orig = blocks
        .iter()
        .map(&original_index)
        .max()
        .map_or(0, |m| m + 1);
    let mut pos: Vec<usize> = vec![usize::MAX; max_orig];

    let mut iter_cap = macro_defs.len().saturating_mul(4) + 32;
    loop {
        iter_cap = iter_cap.saturating_sub(1);
        if iter_cap == 0 {
            break;
        }
        for slot in pos.iter_mut() {
            *slot = usize::MAX;
        }
        for (p, b) in blocks.iter().enumerate() {
            let o = original_index(b);
            if o < pos.len() {
                pos[o] = p;
            }
        }
        let pos_get = |orig: usize| -> usize {
            if orig < pos.len() {
                pos[orig]
            } else {
                usize::MAX
            }
        };

        let mut moved = false;
        for (name, def_orig) in macro_defs {
            let def_pos = pos_get(*def_orig);
            if def_pos == usize::MAX {
                continue;
            }
            let mut min_caller_pos = usize::MAX;
            // (a) non-macro callers in the same scope (transitively expanded)
            if let Some(callers) = callers_of.get(name.as_str()) {
                for &caller_orig in callers {
                    let p = pos_get(caller_orig);
                    if p < min_caller_pos {
                        min_caller_pos = p;
                    }
                }
            }
            // (b) precise per-mod constraint
            if let Some(mods) = mods_of.get(name.as_str()) {
                for &mod_orig in mods {
                    if mod_orig <= *def_orig {
                        continue;
                    }
                    let p = pos_get(mod_orig);
                    if p < min_caller_pos {
                        min_caller_pos = p;
                    }
                }
            }
            // (c) conservative fallback for unreadable child mods
            for &mod_orig in conservative_mods {
                if mod_orig <= *def_orig {
                    continue;
                }
                let p = pos_get(mod_orig);
                if p < min_caller_pos {
                    min_caller_pos = p;
                }
            }
            if min_caller_pos < def_pos {
                let block = blocks.remove(def_pos);
                blocks.insert(min_caller_pos, block);
                moved = true;
                break; // restart with refreshed position map
            }
        }
        if !moved {
            break;
        }
    }
}
