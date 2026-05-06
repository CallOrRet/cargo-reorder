//! Macro placement: every macro item is a hard barrier.
//!
//! `macro_rules!` is textually scoped — moving a definition past one of
//! its callers (or vice versa) breaks compilation, and the call sites
//! can hide in many places (struct-field-init position, type
//! annotations, deeply nested in sibling files reached through
//! `#[macro_use] mod`, etc.). Earlier versions of this crate tried to
//! detect callers and yank definitions back to their first caller; that
//! approach repeatedly missed corner cases (see git history for the
//! collected scars).
//!
//! This module trades sort quality for correctness: every macro-related
//! top-level item becomes a barrier, pinning it in its source position
//! and forbidding any other item from reordering across it. The set of
//! barriers:
//!
//!   * `Item::Macro` with an ident — `macro_rules! foo`. Its
//!     visibility extends downward through the rest of the file and
//!     into child mods declared below it; pinning it preserves that.
//!   * `Item::Macro` without an ident — bare `lazy_static! { ... }` /
//!     `to_hash_map!(...)` style invocations that expand to items at
//!     exactly that location.
//!   * `Item::Mod` carrying `#[macro_use]` — its child's exported
//!     `macro_rules!` leak into this scope from this point downward.
//!
//! Files without any of these items are unaffected; everything else
//! sorts by category as before.
//!
//! [`compute_segments`] is the only public entry point. Items in the
//! same even segment can be reordered freely against each other; a
//! barrier sits on its own private odd segment between them.
//!
//! See README for the deliberate trade-off this represents.

use syn::Item;

/// Assign each item a non-decreasing segment number. Barrier items
/// (see module docs) each get their own private odd segment;
/// everything else stays on the surrounding even segment. Items in
/// different segments will never be reordered past each other by
/// the sort pass.
pub(crate) fn compute_segments(items: &[Item]) -> Vec<u32> {
    let mut barriers_seen = 0u32;
    items
        .iter()
        .map(|item| {
            if is_barrier(item) {
                let s = barriers_seen * 2 + 1;
                barriers_seen += 1;
                s
            } else {
                barriers_seen * 2
            }
        })
        .collect()
}

fn is_barrier(item: &Item) -> bool {
    match item {
        Item::Macro(_) => true,
        Item::Mod(m) => has_macro_use_attr(&m.attrs),
        _ => false,
    }
}

fn has_macro_use_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("macro_use"))
}
