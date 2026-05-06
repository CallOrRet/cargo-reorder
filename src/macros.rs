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
use std::path::{Path, PathBuf};

use std::str::FromStr;

use proc_macro2::{TokenStream, TokenTree};
use syn::Item;

/// Collect `Ident !` pairs whose ident is in `names`. Recurses into
/// delimited groups so calls inside `impl` blocks, fn bodies, etc.
/// are found.
///
/// Path-qualified calls (`crate::foo!()`, `other_crate::foo!()`,
/// `<T>::foo!()`) are skipped: a path-qualified macro call is name-
/// resolved and doesn't constrain the textual order of any local
/// `macro_rules!`. The path-qualified marker is **two** consecutive
/// `:` puncts immediately before the ident (the `::` separator). A
/// single `:` is *not* a marker — that's the struct-field-init or
/// type-annotation syntax, e.g. `Foo { expr: bar!(...) }`, which is
/// a perfectly normal local macro call site (regression: an earlier
/// version checked only one position back and silently swallowed
/// every `field: macro!(...)` use, observed on pest's
/// optimizer/restorer.rs `box_tree!` test fixtures).
pub(crate) fn find_calls(ts: TokenStream, names: &HashSet<String>, found: &mut HashSet<String>) {
    let toks: Vec<TokenTree> = ts.into_iter().collect();
    for i in 0..toks.len() {
        if let TokenTree::Ident(id) = &toks[i] {
            if let Some(TokenTree::Punct(p)) = toks.get(i + 1) {
                if p.as_char() == '!' {
                    let prev_is_colon = i
                        .checked_sub(1)
                        .and_then(|j| toks.get(j))
                        .map(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ':'))
                        .unwrap_or(false);
                    let prev2_is_colon = i
                        .checked_sub(2)
                        .and_then(|j| toks.get(j))
                        .map(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ':'))
                        .unwrap_or(false);
                    let path_qualified = prev_is_colon && prev2_is_colon;
                    if !path_qualified {
                        let n = id.to_string();
                        if names.contains(&n) {
                            found.insert(n);
                        }
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
/// ignored. For `use X as Y;` we record `Y` (the in-scope name).
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

/// Names this `use` tree references *as source* (the leftmost ident
/// of each leaf, before any rename) that match a local macro. This is
/// the set the `use` statement depends on at parse time — `use foo as
/// bar;` records `foo` even though only `bar` becomes the in-scope
/// name. Used to make a `use` item act as a caller of the local
/// `macro_rules! foo` so the yank pipeline keeps the macro above it.
pub(crate) fn collect_use_source_names(
    tree: &syn::UseTree,
    local_macros: &HashSet<String>,
    out: &mut HashSet<String>,
) {
    use syn::UseTree::*;
    match tree {
        Path(p) => collect_use_source_names(&p.tree, local_macros, out),
        Name(n) => {
            let s = n.ident.to_string();
            if local_macros.contains(&s) {
                out.insert(s);
            }
        }
        Rename(r) => {
            // Source ident, NOT the rename — the rename is the new
            // name introduced into scope, but the constraint we care
            // about is "the macro this use *resolves* to must be
            // defined above us".
            let s = r.ident.to_string();
            if local_macros.contains(&s) {
                out.insert(s);
            }
        }
        Glob(_) => {}
        Group(g) => {
            for inner in &g.items {
                collect_use_source_names(inner, local_macros, out);
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
/// Returns `Some(set)` when the child file's path resolves and the
/// file is readable. The fast path parses the child with `syn` and
/// subtracts any `use ... ::name;` imports from the bare-call set;
/// when parsing fails we fall back to a token-only scan via
/// `proc_macro2::TokenStream::from_str`, which can still recognise
/// `name!()` calls in syntactically broken (but lex-valid) files —
/// notably codegen output, in-progress edits, and `#[cfg]`-gated
/// alternatives that don't all parse on their own. The fallback
/// can't subtract imports (no AST), so it's strictly more
/// conservative than the parsed path but still strictly less
/// conservative than treating every macro as a potential need.
///
/// Returns `None` only when the child path can't be resolved or the
/// file can't be read (or even tokenised). Callers fall back to
/// "every mod constrains every macro" in that case.
pub(crate) fn scan_child(
    mod_dir: &Path,
    m: &syn::ItemMod,
    local_macros: &HashSet<String>,
    item_to_tokens: impl Fn(&Item) -> TokenStream,
) -> Option<HashSet<String>> {
    let candidate = crate::discover::external_mod_file(mod_dir, m)?;
    let src = std::fs::read_to_string(&candidate).ok()?;

    if let Ok(parsed) = syn::parse_file(&src) {
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
        return Some(bare.difference(&imported).cloned().collect());
    }

    // Token-only fallback: parse fails (bad syntax / unstable /
    // codegen-with-placeholders) but the lexer often still works.
    let ts = TokenStream::from_str(&src).ok()?;
    let mut bare: HashSet<String> = HashSet::new();
    find_calls(ts, local_macros, &mut bare);
    Some(bare)
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
///
/// `macro_use_is_barrier` lets the caller demote `#[macro_use] mod`
/// items whose leaked macros are provably *not* used by any sibling
/// in this scope (so the barrier costs sort quality without paying
/// for any safety). Returning `true` for an index keeps it as a
/// barrier (the safe default); `false` demotes it. Bare top-level
/// macro invocations are always barriers and don't consult this
/// closure.
pub(crate) fn compute_segments(
    items: &[Item],
    macro_use_is_barrier: impl Fn(usize) -> bool,
) -> Vec<u32> {
    let mut barriers_seen = 0u32;
    items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let is_barrier = match item {
                Item::Macro(m) => m.ident.is_none(),
                Item::Mod(m) => has_macro_use_attr(&m.attrs) && macro_use_is_barrier(idx),
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

/// Names of `macro_rules!` defined at the top level of `items`.
/// Inline `mod foo { ... }` bodies are *not* recursed into — those
/// macros don't leak past the inline mod boundary unless the inline
/// mod itself carries `#[macro_use]`.
fn top_level_macro_rules_names(items: &[Item]) -> HashSet<String> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Macro(m) => m.ident.as_ref().map(|n| n.to_string()),
            _ => None,
        })
        .collect()
}

/// Decide which `#[macro_use] mod` items in `items` actually need to
/// stay barriers. A `#[macro_use] mod foo;` is *relevant* (returns
/// barrier-needed = true) when:
///
/// * the child file (or inline body) defines at least one
///   `macro_rules! NAME`, AND
/// * some sibling item bare-invokes NAME.
///
/// External mods whose child can't be opened/parsed conservatively
/// stay as barriers (we can't prove they don't leak callers).
///
/// Sibling bare-call detection has to look inside external `mod
/// NAME;` siblings (and their own external sub-mods, recursively)
/// because the call to a `#[macro_use]`-leaked macro may live deep
/// in another file — see `external_sibling_calls_any`.
pub(crate) fn compute_macro_use_relevance(
    items: &[Item],
    item_streams: &[TokenStream],
    mod_dir: Option<&Path>,
) -> HashSet<usize> {
    let mut relevant: HashSet<usize> = HashSet::new();
    for (idx, item) in items.iter().enumerate() {
        let Item::Mod(m) = item else { continue };
        if !has_macro_use_attr(&m.attrs) {
            continue;
        }
        // Determine the set of `macro_rules!` exported from this mod.
        let exported: HashSet<String> = match &m.content {
            // Inline body — read directly.
            Some((_, inner)) => top_level_macro_rules_names(inner),
            // External — open the child file, parse, collect.
            None => {
                let Some(dir) = mod_dir else {
                    relevant.insert(idx);
                    continue;
                };
                let Some(child) = crate::discover::external_mod_file(dir, m) else {
                    relevant.insert(idx);
                    continue;
                };
                let Ok(src) = std::fs::read_to_string(&child) else {
                    relevant.insert(idx);
                    continue;
                };
                let Ok(parsed) = syn::parse_file(&src) else {
                    relevant.insert(idx);
                    continue;
                };
                top_level_macro_rules_names(&parsed.items)
            }
        };
        if exported.is_empty() {
            // No macros leak — barrier carries no safety. Demote.
            continue;
        }
        // Any sibling bare-call of an exported name keeps the barrier.
        // For inline siblings the item's own token stream already
        // contains the body, so a normal `find_calls` covers them. For
        // external `mod NAME;` siblings the token stream is just the
        // declaration — we have to open the child file and recurse
        // (bounded by `MAX_MACRO_USE_SCAN_DEPTH` so a pathological
        // mod tree can't make us walk forever). On IO/parse failure
        // or depth cap, fall back to "needed=true" so the barrier
        // stays in place.
        let mut needed = false;
        // Canonical paths of files already scanned for THIS `#[macro_use]
        // mod`. Shared across all sibling iterations so two siblings that
        // resolve to the same file (via `#[path]` alias or symlink) are
        // scanned only once, and a `#[path]` cycle terminates immediately
        // instead of having to bottom out on the depth cap.
        let mut scan_visited: HashSet<PathBuf> = HashSet::new();
        for (j, item) in items.iter().enumerate() {
            if j == idx {
                continue;
            }
            let mut found: HashSet<String> = HashSet::new();
            find_calls(item_streams[j].clone(), &exported, &mut found);
            if !found.is_empty() {
                needed = true;
                break;
            }
            if let Item::Mod(other) = item {
                if other.content.is_none() {
                    let Some(dir) = mod_dir else {
                        needed = true;
                        break;
                    };
                    let Some(other_path) = crate::discover::external_mod_file(dir, other) else {
                        needed = true;
                        break;
                    };
                    if external_sibling_calls_any(&other_path, &exported, &mut scan_visited, 1) {
                        needed = true;
                        break;
                    }
                }
            }
        }
        if needed {
            relevant.insert(idx);
        }
    }
    relevant
}

fn has_macro_use_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("macro_use"))
}

/// Hard cap on how deep the macro_use sibling scan will descend
/// through `mod foo;` -> child file -> its own `mod bar;` -> ... .
/// This is the only recursive walk in the crate that needs a cap:
/// it's the one bounded by file IO, and it has a safe conservative
/// fallback ("might call any macro, keep the barrier") — capping it
/// just trades a little optimisation for a hard upper bound on IO.
/// Picked generously enough that real Rust crates never trip it;
/// macro_use chains beyond a few hops are vanishingly rare.
const MAX_MACRO_USE_SCAN_DEPTH: u32 = 16;

/// Walk an external sibling mod's file (and its own external sub-mods,
/// recursively up to [`MAX_MACRO_USE_SCAN_DEPTH`]) looking for any
/// bare call of a name in `exported`. Returns `true` on the first
/// match, and also `true` on any IO/parse failure or when the depth
/// cap is hit — in those cases we can't prove the absence of calls,
/// so the caller keeps the `#[macro_use] mod` as a barrier.
///
/// `visited` is the set of canonical file paths already scanned in
/// this search context; it dedupes `#[path]` aliases pointing at the
/// same physical file and breaks `#[path]`-induced cycles up front
/// (without it we'd still bottom out on the depth cap, but we'd burn
/// up to MAX_MACRO_USE_SCAN_DEPTH file opens to do it).
///
/// Inline `mod foo { ... }` children inside the visited file don't need
/// extra recursion because their bodies are already part of the parent
/// `Item`'s token stream (so `find_calls` covers them in one shot).
fn external_sibling_calls_any(
    file: &Path,
    exported: &HashSet<String>,
    visited: &mut HashSet<PathBuf>,
    depth: u32,
) -> bool {
    if depth >= MAX_MACRO_USE_SCAN_DEPTH {
        return true;
    }
    // Canonicalise to dedupe `#[path]` aliases / symlinks. A failure
    // here is treated like a read failure (file probably doesn't
    // exist) — fall back to the conservative "might call anything".
    let canonical = match std::fs::canonicalize(file) {
        Ok(p) => p,
        Err(_) => return true,
    };
    if !visited.insert(canonical.clone()) {
        // Already scanned this file in this search and didn't find a
        // call — repeating the scan would give the same answer.
        return false;
    }
    let Ok(src) = std::fs::read_to_string(&canonical) else {
        return true;
    };
    let Ok(parsed) = syn::parse_file(&src) else {
        return true;
    };
    let mod_dir = crate::discover::mod_directory(&canonical);
    for item in &parsed.items {
        let ts = {
            use syn::__private::ToTokens;
            let mut ts = TokenStream::new();
            item.to_tokens(&mut ts);
            ts
        };
        let mut found: HashSet<String> = HashSet::new();
        find_calls(ts, exported, &mut found);
        if !found.is_empty() {
            return true;
        }
        if let Item::Mod(child) = item {
            if child.content.is_none() {
                let Some(child_path) = crate::discover::external_mod_file(&mod_dir, child) else {
                    return true;
                };
                if external_sibling_calls_any(&child_path, exported, visited, depth + 1) {
                    return true;
                }
            }
        }
    }
    false
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
///
/// Calls whose name is shadowed by a parent-scope `use ... ::name;`
/// import (with `as` rename support) don't constrain the local
/// `macro_rules! name` — the call resolves to the imported one. In
/// well-formed Rust this only matters when the local macro and the
/// imported one have different names AND a renamed `use ... as foo;`
/// brings something else under the same name; the subtraction is
/// defensive but correct.
pub(crate) fn collect_item_uses(
    items: &[Item],
    item_streams: &[TokenStream],
    macro_names: &HashSet<String>,
) -> HashMap<usize, HashSet<String>> {
    let mut item_uses: HashMap<usize, HashSet<String>> = HashMap::new();
    if macro_names.is_empty() {
        return item_uses;
    }
    let mut imported: HashSet<String> = HashSet::new();
    for it in items {
        if let Item::Use(u) = it {
            collect_use_leaves(&u.tree, macro_names, &mut imported);
        }
    }
    for (i, (item, ts)) in items.iter().zip(item_streams).enumerate() {
        let mut found: HashSet<String> = HashSet::new();
        find_calls(ts.clone(), macro_names, &mut found);
        if let Item::Macro(m) = item {
            if let Some(name) = &m.ident {
                found.remove(&name.to_string());
            }
        }
        for name in &imported {
            found.remove(name);
        }
        // `use NAME;` (and re-exports like `pub(crate) use NAME;`)
        // resolve `NAME` against the *textual* macro namespace when
        // `NAME` is a local `macro_rules!` defined in this same scope.
        // The `use` therefore requires the macro definition to come
        // textually above it — make the use a caller so the yank
        // fixpoint pulls the def up. Without this, reordering can move
        // a `use macroname;` above its `macro_rules! macroname` and
        // produce code that no longer compiles (observed on xh's
        // src/formatting/palette.rs).
        if let Item::Use(u) = item {
            collect_use_source_names(&u.tree, macro_names, &mut found);
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
///
/// Returns `true` when the cap was reached without converging — the
/// resulting order is still safe (every move during the loop is
/// monotonic: a macro only ever moves earlier) but may not be optimal.
/// Caller can surface a diagnostic.
pub(crate) fn yank_macro_defs<T>(
    blocks: &mut Vec<T>,
    macro_defs: &[(String, usize)],
    effective_uses: &HashMap<usize, HashSet<String>>,
    mod_constraints: &HashMap<usize, HashSet<String>>,
    conservative_mods: &[usize],
    original_index: impl Fn(&T) -> usize,
) -> bool {
    if macro_defs.is_empty() {
        return false;
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
    let mut cap_hit = false;
    loop {
        iter_cap = iter_cap.saturating_sub(1);
        if iter_cap == 0 {
            cap_hit = true;
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
    cap_hit
}
