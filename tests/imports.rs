use cargo_reorder::reorder_source;

#[test]
fn import_groups_ordered_std_external_crate() {
    let input = "\
use crate::local::Thing;
use serde::Serialize;
use std::fmt::Debug;
";
    let out = reorder_source(input).unwrap();
    let p_std = out.find("use std::fmt::Debug").unwrap();
    let p_ext = out.find("use serde::Serialize").unwrap();
    let p_crate = out.find("use crate::local::Thing").unwrap();
    assert!(p_std < p_ext, "{out}");
    assert!(p_ext < p_crate, "{out}");
}

#[test]
fn blank_line_inserted_between_groups() {
    let input = "use crate::a::B;\nuse serde::S;\nuse std::fmt;\n";
    let out = reorder_source(input).unwrap();
    // Expect a blank line between std/external and external/crate groups.
    assert!(
        out.contains("use std::fmt;\n\nuse serde::S;"),
        "missing std/ext separator:\n{out}"
    );
    assert!(
        out.contains("use serde::S;\n\nuse crate::a::B;"),
        "missing ext/crate separator:\n{out}"
    );
}

#[test]
fn nested_use_paths_classify_by_first_segment() {
    let input = "\
use crate::{a, b::c};
use serde::{Serialize, Deserialize};
use std::{collections::HashMap, fmt};
";
    let out = reorder_source(input).unwrap();
    let p_std = out.find("use std::").unwrap();
    let p_ext = out.find("use serde::").unwrap();
    let p_crate = out.find("use crate::").unwrap();
    assert!(p_std < p_ext);
    assert!(p_ext < p_crate);
}

#[test]
fn blank_line_between_use_and_pub_use() {
    let input = "\
pub use crate::reexport::Foo;
use std::fmt;
";
    let out = reorder_source(input).unwrap();
    // use std::fmt; first, then a blank line, then pub use ...
    assert!(
        out.contains("use std::fmt;\n\npub use crate::reexport::Foo;"),
        "expected blank line between use and pub use:\n{out}"
    );
}

#[test]
fn blank_line_between_extern_crate_and_use() {
    let input = "\
use std::fmt;
extern crate alloc;
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("extern crate alloc;\n\nuse std::fmt;"),
        "expected blank line between extern crate and use:\n{out}"
    );
}

#[test]
fn pub_use_ordered_after_use() {
    let input = "\
pub use crate::reexport::Foo;
use std::fmt;
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("use std::fmt").unwrap() < out.find("pub use crate::reexport::Foo").unwrap(),
        "{out}"
    );
}

#[test]
fn crate_super_self_sub_ordering() {
    // Within the third visual group the order must be crate -> super -> self,
    // and they must NOT be separated by blank lines (single visual group).
    let input = "\
use self::inner::Foo;
use super::Bar;
use crate::module::Baz;
";
    let out = reorder_source(input).unwrap();
    let p_crate = out.find("use crate::module::Baz").unwrap();
    let p_super = out.find("use super::Bar").unwrap();
    let p_self = out.find("use self::inner::Foo").unwrap();
    assert!(p_crate < p_super, "crate must come before super:\n{out}");
    assert!(p_super < p_self, "super must come before self:\n{out}");
    // No blank line between any of them.
    let block = &out[p_crate..p_self];
    assert!(
        !block.contains("\n\n"),
        "no blank lines inside crate-local group:\n{out}"
    );
}

#[test]
fn external_then_crate_local_block() {
    let input = "\
use self::a::A;
use serde::Serialize;
use super::b::B;
use std::fmt;
use crate::c::C;
";
    let out = reorder_source(input).unwrap();
    // Visual order: std, blank, external, blank, crate->super->self.
    assert!(
        out.contains("use std::fmt;\n\nuse serde::Serialize;"),
        "{out}"
    );
    assert!(
        out.contains("use serde::Serialize;\n\nuse crate::c::C;"),
        "{out}"
    );
    // Within the third group: crate then super then self, NO blank lines.
    let i = out.find("use crate::c::C;").unwrap();
    let j = out.find("use self::a::A;").unwrap();
    let block = &out[i..j];
    assert!(!block.contains("\n\n"), "third group must be glued:\n{out}");
}

#[test]
fn local_mod_use_sorts_after_self_in_third_group() {
    // `mod helpers;` makes `use helpers::Thing;` a local-mod import; it must
    // sort after `self::*` and stay glued to it (no blank line).
    let input = "\
use helpers::Thing;
use self::inner::Foo;
use super::Bar;
use crate::module::Baz;
mod helpers;
";
    let out = reorder_source(input).unwrap();
    let p_crate = out.find("use crate::module::Baz").unwrap();
    let p_super = out.find("use super::Bar").unwrap();
    let p_self = out.find("use self::inner::Foo").unwrap();
    let p_local = out.find("use helpers::Thing").unwrap();
    assert!(p_crate < p_super, "{out}");
    assert!(p_super < p_self, "{out}");
    assert!(
        p_self < p_local,
        "self must come before local-mod use:\n{out}"
    );
    // The whole crate-local block has no internal blank line.
    let block = &out[p_crate..p_local];
    assert!(
        !block.contains("\n\n"),
        "no blank line inside crate-local group:\n{out}"
    );
}

#[test]
fn local_mod_falls_back_to_external_when_no_local_decl() {
    // No `mod helpers;` declared, so `use helpers::Thing;` is external.
    let input = "use helpers::Thing;\nuse std::fmt;\n";
    let out = reorder_source(input).unwrap();
    // External stays in second group, separated from std by a blank line.
    assert!(
        out.contains("use std::fmt;\n\nuse helpers::Thing;"),
        "should be classified as external when no local mod:\n{out}"
    );
}

#[test]
fn glob_use_classified_by_first_segment() {
    let input = "\
use crate::*;
use serde::*;
use std::*;
";
    let out = reorder_source(input).unwrap();
    let p_std = out.find("use std::*").unwrap();
    let p_serde = out.find("use serde::*").unwrap();
    let p_crate = out.find("use crate::*").unwrap();
    assert!(p_std < p_serde, "glob std::*  → Std group:\n{out}");
    assert!(
        p_serde < p_crate,
        "glob crate::* → CrateLocal group:\n{out}"
    );
}

#[test]
fn leading_colon_use_classifies_by_first_segment() {
    // `use ::std::fmt::Display;` (Rust 2015 absolute path) — first
    // segment after the leading `::` is `std`, so this is a Std import.
    let input = "\
use ::serde::Serialize;
use ::std::fmt::Display;
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("use ::std::fmt::Display").unwrap() < out.find("use ::serde::Serialize").unwrap(),
        "leading colon doesn't change first-segment classification:\n{out}"
    );
}

#[test]
fn order_within_group_is_stable() {
    // Same group, so original relative order should be preserved.
    let input = "use std::fmt;\nuse std::collections::HashMap;\nuse std::path::Path;\n";
    let out = reorder_source(input).unwrap();
    let p1 = out.find("use std::fmt;").unwrap();
    let p2 = out.find("use std::collections::HashMap;").unwrap();
    let p3 = out.find("use std::path::Path;").unwrap();
    assert!(p1 < p2 && p2 < p3, "{out}");
}
