//! Tests for every `Config` flag (the public CLI options live here too,
//! routed through `reorder_source_with`).

use cargo_reorder::{Config, reorder_source, reorder_source_with};

#[test]
fn default_is_use_first() {
    let input = "use std::fmt;\nmod child;\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("use std::fmt").unwrap() < out.find("mod child").unwrap(),
        "{out}"
    );
}

#[test]
fn mod_before_use_swaps() {
    let input = "use std::fmt;\nmod child;\n";
    let cfg = Config {
        mod_before_use: true,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    assert!(
        out.find("mod child").unwrap() < out.find("use std::fmt").unwrap(),
        "{out}"
    );
}

#[test]
fn no_import_groups_keeps_original_order() {
    let input = "\
use crate::a;
use serde::S;
use std::fmt;
";
    let cfg = Config {
        group_imports: false,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    let p_crate = out.find("use crate::a").unwrap();
    let p_serde = out.find("use serde::S").unwrap();
    let p_std = out.find("use std::fmt").unwrap();
    // Source order preserved (no std/ext/crate sorting).
    assert!(p_crate < p_serde && p_serde < p_std, "{out}");
    // Also no blank lines inserted between them.
    assert!(!out.contains("use crate::a;\n\n"), "{out}");
}

#[test]
fn no_impl_grouping_buckets_all_impls() {
    let input = "\
impl Foo { fn a() {} }
struct Foo;
struct Bar;
impl Bar { fn b() {} }
";
    let cfg = Config {
        group_impls_with_type: false,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    let p_foo = out.find("struct Foo").unwrap();
    let p_bar = out.find("struct Bar").unwrap();
    let p_impl_foo = out.find("impl Foo").unwrap();
    let p_impl_bar = out.find("impl Bar").unwrap();
    // Without grouping: all structs first, then all impls in source order.
    assert!(
        p_foo < p_bar && p_bar < p_impl_foo && p_impl_foo < p_impl_bar,
        "{out}"
    );
}

#[test]
fn no_tests_last_keeps_test_mod_in_place() {
    let input = "\
fn before() {}

#[cfg(test)]
mod tests { #[test] fn t() {} }

fn after() {}
";
    let cfg = Config {
        tests_last: false,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    // With tests_last=false, the test mod sorts as a normal Mod (weight
    // 30, near top), not at end.
    assert!(
        out.find("mod tests").unwrap() < out.find("fn before").unwrap(),
        "tests_last=false: mod tests should sort like a normal mod:\n{out}"
    );
}

#[test]
fn pub_mod_first_groups_them_with_blank_line() {
    let input = "\
mod b;
pub mod a;
mod d;
pub mod c;
";
    let cfg = Config {
        pub_mod_first: true,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    let p_pa = out.find("pub mod a").unwrap();
    let p_pc = out.find("pub mod c").unwrap();
    let p_b = out.find("mod b;").unwrap();
    let p_d = out.find("mod d;").unwrap();
    assert!(p_pa < p_pc && p_pc < p_b && p_b < p_d, "{out}");
    // Blank line between pub-mod group and private-mod group.
    let between = &out[p_pc..p_b];
    assert!(
        between.contains("\n\n"),
        "expected blank line between groups:\n{out}"
    );
}

#[test]
fn pub_mod_first_off_preserves_blank_lines_between_mods() {
    // Default (pub_mod_first off) must not shuffle mod order AND must
    // preserve blank lines from the source (they live in `trailing`).
    let input = "\
mod a;

mod b;
mod c;

pub mod d;
fn x() {}
";
    let out = reorder_source(input).unwrap();
    // Source structure preserved exactly through the mod region.
    assert!(
        out.contains("mod a;\n\nmod b;\nmod c;\n\npub mod d;"),
        "default mode must preserve mod blank lines verbatim:\n{out}"
    );
}

#[test]
fn pub_mod_first_off_keeps_interleaved_order() {
    let input = "pub mod a;\nmod b;\npub mod c;\nmod d;\n";
    let out = reorder_source(input).unwrap();
    let p_a = out.find("pub mod a").unwrap();
    let p_b = out.find("mod b;").unwrap();
    let p_c = out.find("pub mod c").unwrap();
    let p_d = out.find("mod d;").unwrap();
    assert!(
        p_a < p_b && p_b < p_c && p_c < p_d,
        "default: keep interleaved:\n{out}"
    );
    let block = &out[p_a..=p_d];
    assert!(
        !block.contains("\n\n"),
        "no blank line inside mod block in default:\n{out}"
    );
}

#[test]
fn default_is_trait_before_struct() {
    let input = "struct S;\ntrait T {}\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("trait T").unwrap() < out.find("struct S").unwrap(),
        "default: trait < struct:\n{out}"
    );
}

#[test]
fn struct_before_trait_flag_swaps() {
    let input = "trait T {}\nstruct S;\n";
    let cfg = Config {
        struct_before_trait: true,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    assert!(
        out.find("struct S").unwrap() < out.find("trait T").unwrap(),
        "with flag: struct < trait:\n{out}"
    );
}

#[test]
fn default_lifts_trait_above_enum_too() {
    let input = "enum E { A }\nstruct S;\ntrait T {}\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("trait T").unwrap() < out.find("enum E").unwrap(),
        "{out}"
    );
    assert!(
        out.find("enum E").unwrap() < out.find("struct S").unwrap(),
        "{out}"
    );
}
