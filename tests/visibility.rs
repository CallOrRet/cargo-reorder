//! Visibility variants: `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`,
//! and inherited (private). The reorderer's category logic uses
//! visibility only to distinguish `Use` from `PubUse`; all other items
//! sort by category irrespective of visibility, but the visibility
//! token itself must round-trip exactly.

use cargo_reorder::reorder_source;

#[test]
fn pub_in_path_round_trips() {
    let input = "fn z() {}\npub(in crate::shared) struct Restricted;\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("pub(in crate::shared) struct Restricted"),
        "{out}"
    );
}

#[test]
fn pub_super_fn_round_trips() {
    let input = "struct S;\npub(super) fn helper() {}\nstruct T;\n";
    let out = reorder_source(input).unwrap();
    assert!(out.contains("pub(super) fn helper"), "{out}");
}

#[test]
fn pub_crate_struct_round_trips() {
    let input = "fn z() {}\npub(crate) struct Internal { f: u32 }\n";
    let out = reorder_source(input).unwrap();
    assert!(out.contains("pub(crate) struct Internal"), "{out}");
    assert!(
        out.find("pub(crate) struct").unwrap() < out.find("fn z").unwrap(),
        "{out}"
    );
}

#[test]
fn pub_use_distinguished_from_use() {
    let input = "\
pub use crate::reexport::Foo;
use std::fmt;
pub use crate::another::Bar;
use serde::Serialize;
";
    let out = reorder_source(input).unwrap();
    let p_use_std = out.find("use std::fmt").unwrap();
    let p_use_serde = out.find("use serde::Serialize").unwrap();
    let p_pub_use_foo = out.find("pub use crate::reexport::Foo").unwrap();
    let p_pub_use_bar = out.find("pub use crate::another::Bar").unwrap();
    // All `use` come first, then all `pub use` (in source order).
    assert!(p_use_std < p_pub_use_foo, "{out}");
    assert!(p_use_serde < p_pub_use_foo, "{out}");
    assert!(
        p_pub_use_foo < p_pub_use_bar,
        "pub use group preserves source order:\n{out}"
    );
    // Within `use`: std before external (by group), preserving order.
    assert!(p_use_std < p_use_serde, "{out}");
}

#[test]
fn pub_crate_use_classifies_as_pub_use() {
    // `pub(crate) use ...` is still a re-export — it just narrows the
    // visibility. Should sort with the PubUse bucket.
    let input = "\
fn z() {}
pub(crate) use crate::internal::Helper;
use std::fmt;
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("use std::fmt").unwrap()
            < out.find("pub(crate) use crate::internal::Helper").unwrap(),
        "pub(crate) use should sort with pub use, after plain use:\n{out}"
    );
}

#[test]
fn vis_on_mod_kept_with_mod_decl() {
    let input = "\
fn z() {}
pub mod public_api;
mod internal;
pub(crate) mod restricted;
";
    let out = reorder_source(input).unwrap();
    // All mods are weight 30 → sort before fn (weight 90), preserving
    // their relative source order within the Mod bucket.
    let p_public = out.find("pub mod public_api").unwrap();
    let p_internal = out.find("mod internal").unwrap();
    let p_restricted = out.find("pub(crate) mod restricted").unwrap();
    let p_fn = out.find("fn z").unwrap();
    assert!(p_public < p_internal && p_internal < p_restricted, "{out}");
    assert!(p_restricted < p_fn, "{out}");
}
