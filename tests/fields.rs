//! Field-group-sort feature: groups named fields/variants by their
//! first word (snake_case `_` separator OR camelCase / PascalCase
//! boundary), sorts within group by name length, and orders groups
//! by the group's mean name length, with a blank line between
//! groups. Default ON; opt-out with `Config { no_reorder_fields:
//! true, ..default }`.
//!
//! Skip rules (item left untouched):
//! - any `#[repr(...)]`
//! - `#[derive(PartialOrd)]` or `#[derive(Ord)]`
//! - enum with any explicit discriminant (`A = 1`)
//! - tuple / unit fields
//! - inline (single-line) layouts
//! - <2 fields/variants

use cargo_reorder::{Config, reorder_source, reorder_source_with};

fn opt_out() -> Config {
    Config {
        no_reorder_fields: true,
        ..Config::default()
    }
}

#[test]
fn struct_groups_by_snake_prefix_and_sorts_by_length() {
    let input = "\
struct S {
    foo_loooong: String,
    bar_x: u8,
    foo_short: u8,
    foo_medium: bool,
    bar_y: u32,
}
";
    let out = reorder_source(input).unwrap();
    let want = "\
struct S {
    bar_x: u8,
    bar_y: u32,

    foo_short: u8,
    foo_medium: bool,
    foo_loooong: String,
}
";
    assert_eq!(out, want, "got:\n{out}");
}

#[test]
fn enum_groups_by_pascal_prefix() {
    let input = "\
enum E {
    BarBanana,
    FooLong,
    Foo,
    BarApple,
    FooMedium,
}
";
    let out = reorder_source(input).unwrap();
    let want = "\
enum E {
    Foo,
    FooLong,
    FooMedium,

    BarApple,
    BarBanana,
}
";
    assert_eq!(out, want, "got:\n{out}");
}

#[test]
fn within_group_ties_preserve_source_order() {
    let input = "\
struct S {
    bar_y: u32,
    bar_x: u8,
}
";
    // Both 5 chars — tie → source order preserved (bar_y before bar_x).
    let out = reorder_source(input).unwrap();
    let p_y = out.find("bar_y").unwrap();
    let p_x = out.find("bar_x").unwrap();
    assert!(p_y < p_x, "tie should preserve source order:\n{out}");
}

#[test]
fn between_group_ties_preserve_source_order() {
    // Both groups have one 3-char member → both have mean 3 → tie;
    // bar appeared first in source so its group comes first.
    let input = "\
struct S {
    bar: u32,
    foo: u8,
}
";
    let out = reorder_source(input).unwrap();
    let p_bar = out.find("bar:").unwrap();
    let p_foo = out.find("foo:").unwrap();
    assert!(p_bar < p_foo, "{out}");
    // Different prefixes => blank line between them.
    let between = &out[p_bar..p_foo];
    assert!(
        between.contains("\n\n"),
        "blank line between groups:\n{out}"
    );
}

#[test]
fn doc_comments_travel_with_their_field() {
    let input = "\
struct S {
    /// docs for foo_z
    foo_z: u8,
    /// docs for bar
    bar: u32,
    /// docs for foo_a
    foo_a: bool,
}
";
    let out = reorder_source(input).unwrap();
    // Ensure each `///` line is immediately followed by its field.
    assert!(out.contains("/// docs for bar\n    bar: u32,"), "{out}");
    assert!(out.contains("/// docs for foo_z\n    foo_z: u8,"), "{out}");
    assert!(
        out.contains("/// docs for foo_a\n    foo_a: bool,"),
        "{out}"
    );
}

#[test]
fn attributes_travel_with_their_field() {
    let input = "\
struct S {
    #[serde(default)]
    foo_z: u8,
    bar: u32,
    #[deprecated]
    foo_a: bool,
}
";
    let out = reorder_source(input).unwrap();
    assert!(out.contains("#[serde(default)]\n    foo_z: u8,"), "{out}");
    assert!(out.contains("#[deprecated]\n    foo_a: bool,"), "{out}");
}

#[test]
fn opt_out_preserves_source_order() {
    let input = "\
struct S {
    foo_loooong: String,
    bar_x: u8,
}
";
    let out = reorder_source_with(input, &opt_out()).unwrap();
    assert_eq!(out, input);
}

#[test]
fn repr_c_skipped() {
    let input = "\
#[repr(C)]
struct S {
    foo_loooong: String,
    bar_x: u8,
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input, "repr(C) must not reorder fields:\n{out}");
}

#[test]
fn repr_packed_skipped() {
    let input = "\
#[repr(packed)]
struct S {
    foo_loooong: String,
    bar_x: u8,
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn repr_transparent_skipped() {
    let input = "\
#[repr(transparent)]
struct S {
    foo_loooong: String,
    bar_x: u8,
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn derive_ord_skipped() {
    let input = "\
#[derive(Ord, PartialOrd, Eq, PartialEq)]
struct S {
    foo_loooong: String,
    bar_x: u8,
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input, "derive(Ord) must not reorder:\n{out}");
}

#[test]
fn derive_partial_ord_skipped() {
    let input = "\
#[derive(PartialOrd, PartialEq)]
struct S {
    foo_loooong: String,
    bar_x: u8,
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn derive_other_traits_does_not_skip() {
    // Debug/Clone don't depend on field order — should still reorder.
    let input = "\
#[derive(Debug, Clone)]
struct S {
    foo_loooong: String,
    bar_x: u8,
}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("bar_x").unwrap() < out.find("foo_loooong").unwrap(),
        "{out}"
    );
}

#[test]
fn enum_with_discriminant_skipped() {
    let input = "\
enum E {
    Foo = 1,
    BarBanana,
    BarApple,
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(
        out, input,
        "enum with explicit discriminant must not reorder:\n{out}"
    );
}

#[test]
fn enum_repr_int_skipped() {
    let input = "\
#[repr(u8)]
enum E {
    Foo,
    BarBanana,
    BarApple,
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn tuple_struct_unaffected() {
    let input = "struct T(u32, String, u8);\n";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn unit_struct_unaffected() {
    let input = "struct U;\n";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn single_field_struct_unaffected() {
    let input = "\
struct S {
    only: u8,
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn empty_struct_body_unaffected() {
    let input = "struct E {}\n";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn inline_single_line_struct_unaffected() {
    // All fields on one line — line-based slicing can't disentangle
    // them, so we deliberately leave it alone.
    let input = "struct S { foo_long: u8, bar: u32 }\n";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn multi_line_struct_variant_in_enum_reorders_inner_fields() {
    let input = "\
enum E {
    BarThing {
        foo_z: u8,
        bar: u32,
        foo_a: bool,
    },
    FooThing,
}
";
    let out = reorder_source(input).unwrap();
    // Inner fields of BarThing should be regrouped: bar, then foo_z, foo_a.
    let p_bar = out.find("bar: u32,").unwrap();
    let p_foo_z = out.find("foo_z").unwrap();
    let p_foo_a = out.find("foo_a").unwrap();
    assert!(p_bar < p_foo_z && p_foo_z < p_foo_a, "{out}");
}

#[test]
fn inline_struct_variant_inner_left_alone() {
    let input = "\
enum E {
    BarThing { foo_z: u8, bar: u32, foo_a: bool },
    FooThing,
}
";
    let out = reorder_source(input).unwrap();
    // Inner fields stay in source order on the single line.
    assert!(
        out.contains("BarThing { foo_z: u8, bar: u32, foo_a: bool }"),
        "{out}"
    );
}

#[test]
fn union_fields_reorder_when_safe() {
    // Plain union without #[repr] — though unions are usually used
    // with #[repr(C)], a `union` without it (rare) is reorderable.
    // Most real unions DO have repr; this just covers the code path.
    let input = "\
union U {
    foo_loooong: u64,
    bar: u8,
    foo_short: u32,
}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("bar:").unwrap() < out.find("foo_short").unwrap(),
        "{out}"
    );
}

#[test]
fn union_with_repr_c_skipped() {
    let input = "\
#[repr(C)]
union U {
    foo_loooong: u64,
    bar: u8,
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn idempotent_after_one_pass() {
    let input = "\
struct S {
    foo_z: u8,
    bar_x: u32,
    foo_a: bool,
    bar_y: i64,
}
";
    let pass1 = reorder_source(input).unwrap();
    let pass2 = reorder_source(&pass1).unwrap();
    assert_eq!(
        pass1, pass2,
        "second pass must be a no-op:\n{pass1}\n---\n{pass2}"
    );
}

#[test]
fn group_order_uses_mean_length_not_first_member() {
    // Group `aa`: aa_x (4), aa_loooong (9) → mean 6.5, first member 4.
    // Group `bb`: bb_xx (5), bb_yy (5) → mean 5, first member 5.
    //
    // If groups were ordered by *first member's length*, `aa` (4)
    // would precede `bb` (5). Under the mean rule, `bb` (mean 5)
    // precedes `aa` (mean 6.5). This test pins the mean behaviour.
    let input = "\
struct S {
    aa_x: u8,
    aa_loooong: String,
    bb_xx: u8,
    bb_yy: u8,
}
";
    let out = reorder_source(input).unwrap();
    let p_bb = out.find("bb_xx").unwrap();
    let p_aa = out.find("aa_x").unwrap();
    assert!(
        p_bb < p_aa,
        "group `bb` (mean 5) must precede group `aa` (mean 6.5):\n{out}"
    );
}

#[test]
fn nameless_prefix_each_in_own_group() {
    // Field names with no `_` and no PascalCase boundary form
    // single-name groups — each is its own group, so each gets a
    // blank line between it and its neighbours.
    let input = "\
struct S {
    a: u8,
    bb: u16,
    c: u32,
}
";
    let out = reorder_source(input).unwrap();
    // Groups: a (1), c (1), bb (2). Three groups, sorted by length:
    // a (1), c (1), bb (2). Order between a and c: source order
    // preserved (a before c). Then bb.
    let p_a = out.find("a: u8").unwrap();
    let p_c = out.find("c: u32").unwrap();
    let p_b = out.find("bb: u16").unwrap();
    assert!(p_a < p_c && p_c < p_b, "{out}");
    // Blank lines between each group.
    let block = &out[p_a..=p_b + 7];
    let blanks = block.matches("\n\n").count();
    assert!(blanks >= 2, "expected >= 2 blank-line separators:\n{out}");
}
