//! Grouping logic for `use` and `pub use` items.
//!
//! Three visual groups, separated by a single blank line in the output:
//!
//! * **std**: `std::*`, `core::*`, `alloc::*`
//! * **external**: third-party crate paths
//! * **crate-local**: `crate::*` → `super::*` → `self::*` →
//!   `<local_mod>::*` (any path whose first segment matches a `mod foo;`
//!   declared at the top level of the same file)
//!
//! `ImportGroup` carries the fine-grained kind for sort ordering;
//! `visual()` collapses the crate-local kinds back to one bucket so the
//! blank-line separator logic does not split them apart.

use std::collections::HashSet;

use syn::{ItemUse, UseTree};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImportGroup {
    Std,
    External,
    Crate,
    Super,
    Self_,
    /// Path whose first segment is a `mod foo;` declared in the same file.
    LocalMod,
}

impl ImportGroup {
    pub fn visual(self) -> VisualGroup {
        match self {
            ImportGroup::Std => VisualGroup::Std,
            ImportGroup::External => VisualGroup::External,
            ImportGroup::Crate
            | ImportGroup::Super
            | ImportGroup::Self_
            | ImportGroup::LocalMod => VisualGroup::CrateLocal,
        }
    }

    /// Classify a `use` item. `local_mods` is the set of module names declared
    /// at the top level of the same file (so `mod foo;` makes
    /// `use foo::Bar;` resolve to a local module rather than an external
    /// crate).
    pub fn classify(item: &ItemUse, local_mods: &HashSet<String>) -> Self {
        let mut tree = &item.tree;
        if let UseTree::Path(p) = tree {
            if item.leading_colon.is_some() {
                return classify_ident(&p.ident.to_string(), local_mods);
            }
        }
        loop {
            match tree {
                UseTree::Path(p) => return classify_ident(&p.ident.to_string(), local_mods),
                UseTree::Name(n) => return classify_ident(&n.ident.to_string(), local_mods),
                UseTree::Rename(r) => return classify_ident(&r.ident.to_string(), local_mods),
                UseTree::Glob(_) => return ImportGroup::External,
                UseTree::Group(g) => {
                    if let Some(first) = g.items.first() {
                        tree = first;
                        continue;
                    }
                    return ImportGroup::External;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualGroup {
    Std,
    External,
    CrateLocal,
}

fn classify_ident(ident: &str, local_mods: &HashSet<String>) -> ImportGroup {
    match ident {
        "std" | "core" | "alloc" => ImportGroup::Std,
        "crate" => ImportGroup::Crate,
        "super" => ImportGroup::Super,
        "self" => ImportGroup::Self_,
        other if local_mods.contains(other) => ImportGroup::LocalMod,
        _ => ImportGroup::External,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn no_mods() -> HashSet<String> {
        HashSet::new()
    }

    fn mods(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn classifies_std() {
        let it: ItemUse = parse_quote!(
            use std::fmt::Debug;
        );
        assert_eq!(ImportGroup::classify(&it, &no_mods()), ImportGroup::Std);
        let it: ItemUse = parse_quote!(
            use core::mem;
        );
        assert_eq!(ImportGroup::classify(&it, &no_mods()), ImportGroup::Std);
        let it: ItemUse = parse_quote!(
            use alloc::vec::Vec;
        );
        assert_eq!(ImportGroup::classify(&it, &no_mods()), ImportGroup::Std);
    }

    #[test]
    fn classifies_crate_super_self_distinctly() {
        let c: ItemUse = parse_quote!(
            use crate::foo::Bar;
        );
        let s: ItemUse = parse_quote!(
            use super::Thing;
        );
        let f: ItemUse = parse_quote!(
            use self::inner::*;
        );
        assert_eq!(ImportGroup::classify(&c, &no_mods()), ImportGroup::Crate);
        assert_eq!(ImportGroup::classify(&s, &no_mods()), ImportGroup::Super);
        assert_eq!(ImportGroup::classify(&f, &no_mods()), ImportGroup::Self_);
        for it in [&c, &s, &f] {
            assert_eq!(
                ImportGroup::classify(it, &no_mods()).visual(),
                VisualGroup::CrateLocal
            );
        }
    }

    #[test]
    fn classifies_external() {
        let it: ItemUse = parse_quote!(
            use serde::Serialize;
        );
        assert_eq!(
            ImportGroup::classify(&it, &no_mods()),
            ImportGroup::External
        );
    }

    #[test]
    fn local_mod_overrides_external_classification() {
        let it: ItemUse = parse_quote!(
            use helpers::Thing;
        );
        // No `mod helpers;` in the file → external.
        assert_eq!(
            ImportGroup::classify(&it, &no_mods()),
            ImportGroup::External
        );
        // With `mod helpers;` declared locally → LocalMod.
        assert_eq!(
            ImportGroup::classify(&it, &mods(&["helpers"])),
            ImportGroup::LocalMod
        );
        // LocalMod still belongs to the same visual group as crate/super/self.
        assert_eq!(
            ImportGroup::classify(&it, &mods(&["helpers"])).visual(),
            VisualGroup::CrateLocal
        );
    }
}
