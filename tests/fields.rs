//! Field-group-sort feature: groups named fields/variants by their
//! first word (snake_case `_` separator OR camelCase / PascalCase
//! boundary), sorts within group by name length, and orders groups
//! by the group's mean name length, with a blank line between
//! groups. Default ON; opt-out with `Config { no_fields:
//! true, ..default }`.
//!
//! Skip rules (item left untouched):
//! - any `#[repr(...)]`
//! - `#[derive(PartialOrd)]` or `#[derive(Ord)]`
//! - enum with any explicit discriminant (`A = 1`)
//! - tuple / unit fields
//! - <2 fields/variants

use cargo_reorder::{Config, reorder_source, reorder_source_with};

fn opt_out() -> Config {
    Config {
        no_fields: true,
        ..Config::default()
    }
}

fn single_line_off() -> Config {
    Config {
        no_single_line_fields: true,
        ..Config::default()
    }
}

fn fn_args_on() -> Config {
    Config {
        fn_args: true,
        ..Config::default()
    }
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
fn unit_struct_unaffected() {
    let input = "struct U;\n";
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
fn empty_struct_body_unaffected() {
    let input = "struct E {}\n";
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
fn cfg_attr_with_derive_ord_skipped() {
    // `#[cfg_attr(feature = "x", derive(Ord, PartialOrd))]` pins
    // field order under the matching cfg, so reorder must skip
    // unconditionally — we don't evaluate the cfg.
    let input = "\
#[cfg_attr(feature = \"cmp\", derive(Ord, PartialOrd, Eq, PartialEq))]
struct S {
    foo_loooong: String,
    bar_x: u8,
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input, "cfg_attr derive(Ord) must not reorder:\n{out}");
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
fn enum_with_multiline_cfg_attr_keeps_attributes_intact() {
    // Regression: rewrite_enum step 2 used to pass the original source
    // start_line as text_start_line when re-parsing the modified body.
    // The re-parse spans are 1-indexed within the new string, so the
    // mismatch caused sort_top_level to confuse attribute lines for
    // variant lines. Output became unparseable. Pin the fixed
    // behaviour here.
    let input = "\
//! header

use crate::Whatever;

#[derive(Debug, Clone, Copy)]
#[cfg_attr(
    feature = \"bevy_reflect\",
    derive(Reflect),
    reflect(Clone, PartialEq, Default)
)]
#[cfg_attr(feature = \"serialize\", derive(serde::Serialize, serde::Deserialize))]
pub enum Color {
    /// A color in the sRGB color space with alpha.
    Srgba(Srgba),
    /// A color in the linear sRGB color space with alpha.
    LinearRgba(LinearRgba),
    /// A color in the HSL color space with alpha.
    Hsla(Hsla),
}
";
    let out = reorder_source(input).unwrap();
    // Output must still parse — no attribute lines escaping the header.
    syn::parse_file(&out).expect("output should parse");
    // The multi-line cfg_attr block must survive in one piece, in the
    // header (before `pub enum Color {`), in the original order.
    let cfg_pos = out
        .find("#[cfg_attr(\n    feature = \"bevy_reflect\",")
        .expect("cfg_attr should still be intact above the enum");
    let enum_pos = out.find("pub enum Color {").unwrap();
    assert!(
        cfg_pos < enum_pos,
        "cfg_attr must precede enum body:\n{out}"
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
fn inline_single_line_struct_reorders_by_default() {
    let input = "struct S { foo_long: u8, bar: u32 }\n";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, "struct S { bar: u32, foo_long: u8 }\n");
}

#[test]
fn inline_struct_variant_inner_reorders_by_default() {
    let input = "\
enum E {
    BarThing { foo_z: u8, bar: u32, foo_a: bool },
    FooThing,
}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("BarThing { bar: u32, foo_z: u8, foo_a: bool }"),
        "{out}"
    );
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
fn struct_init_fields_reorder() {
    let input = "\
struct S {
    bar: u32,
    foo_z: u8,
    foo_a: bool,
}

fn make() -> S {
    S {
        foo_z: 1,
        bar: 2,
        foo_a: true,
    }
}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("S {\n        bar: 2,\n\n        foo_z: 1,\n        foo_a: true,\n    }"),
        "{out}"
    );
}

#[test]
fn enum_variant_init_fields_reorder() {
    let input = "\
enum E {
    V {
        bar: u32,
        foo_z: u8,
        foo_a: bool,
    },
}

fn make() -> E {
    E::V {
        foo_z: 1,
        bar: 2,
        foo_a: true,
    }
}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("E::V {\n        bar: 2,\n\n        foo_z: 1,\n        foo_a: true,\n    }"),
        "{out}"
    );
}

#[test]
fn union_init_fields_reorder() {
    let input = "\
union U {
    bar: u32,
    foo_z: u8,
    foo_a: bool,
}

fn make() -> U {
    U {
        foo_z: 1,
        bar: 2,
        foo_a: true,
    }
}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("U {\n        bar: 2,\n\n        foo_z: 1,\n        foo_a: true,\n    }"),
        "{out}"
    );
}

#[test]
fn struct_init_shorthand_and_rest_reorder_fields_only() {
    let input = "\
struct S {
    bar: u32,
    foo_z: u8,
    foo_a: bool,
}

fn make(base: S, foo_z: u8, bar: u32, foo_a: bool) -> S {
    S {
        foo_z,
        bar,
        foo_a,
        ..base
    }
}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("S {\n        bar,\n\n        foo_z,\n        foo_a,\n        ..base\n    }"),
        "{out}"
    );
}

#[test]
fn nested_struct_init_fields_reorder_inside_and_outside() {
    let input = "\
struct Inner {
    bar: u32,
    foo_z: u8,
    foo_a: bool,
}

struct Outer {
    bar: u32,
    foo_z: Inner,
    foo_a: bool,
}

fn make() -> Outer {
    Outer {
        foo_z: Inner {
            foo_z: 1,
            bar: 2,
            foo_a: true,
        },
        bar: 3,
        foo_a: false,
    }
}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains(
            "Outer {\n        bar: 3,\n\n        foo_z: Inner {\n            bar: 2,\n\n            foo_z: 1,\n            foo_a: true,\n        },\n        foo_a: false,\n    }"
        ),
        "{out}"
    );
}

#[test]
fn opt_out_preserves_struct_init_order() {
    let input = "\
struct S {
    bar: u32,
    foo_z: u8,
    foo_a: bool,
}

fn make() -> S {
    S {
        foo_z: 1,
        bar: 2,
        foo_a: true,
    }
}
";
    let out = reorder_source_with(input, &opt_out()).unwrap();
    assert_eq!(out, input);
}

#[test]
fn struct_init_multi_group_exact_output() {
    let input = "\
struct S {
    a: u8,
    bb: u8,
    user_id: u64,
    user_name: String,
    cache_path: String,
}

fn make() -> S {
    S {
        cache_path: String::new(),
        user_name: String::new(),
        bb: 2,
        user_id: 1,
        a: 0,
    }
}
";
    let want = "\
struct S {
    a: u8,

    bb: u8,

    user_id: u64,
    user_name: String,

    cache_path: String,
}

fn make() -> S {
    S {
        a: 0,

        bb: 2,

        user_id: 1,
        user_name: String::new(),

        cache_path: String::new(),
    }
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, want);
}

#[test]
fn struct_init_comments_travel_with_fields() {
    let input = "\
struct S {
    bar: u32,
    foo_z: u8,
    foo_a: bool,
}

fn make() -> S {
    S {
        // z comment
        foo_z: 1,
        // bar comment
        bar: 2,
        // a comment
        foo_a: true,
    }
}
";
    let want = "\
struct S {
    bar: u32,

    foo_z: u8,
    foo_a: bool,
}

fn make() -> S {
    S {
        // bar comment
        bar: 2,

        // z comment
        foo_z: 1,
        // a comment
        foo_a: true,
    }
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, want);
}

#[test]
fn struct_init_inside_call_and_array_reorders() {
    let input = "\
struct S {
    bar: u32,
    foo_z: u8,
    foo_a: bool,
}

fn make() {
    consume(S {
        foo_z: 1,
        bar: 2,
        foo_a: true,
    });
    let _items = [S {
        foo_z: 3,
        bar: 4,
        foo_a: false,
    }];
}
";
    let want = "\
struct S {
    bar: u32,

    foo_z: u8,
    foo_a: bool,
}

fn make() {
    consume(S {
        bar: 2,

        foo_z: 1,
        foo_a: true,
    });
    let _items = [S {
        bar: 4,

        foo_z: 3,
        foo_a: false,
    }];
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, want);
}

#[test]
fn no_single_line_fields_keeps_struct_init_order() {
    let input = "\
struct S {
    bar: u32,
    foo_long: u8,
}

fn make() -> S {
    S { foo_long: 1, bar: 2 }
}
";
    let want = "\
struct S {
    bar: u32,

    foo_long: u8,
}

fn make() -> S {
    S { foo_long: 1, bar: 2 }
}
";
    let out = reorder_source_with(input, &single_line_off()).unwrap();
    assert_eq!(out, want);
}

#[test]
fn single_line_reorders_struct_union_and_enum_definitions_by_default() {
    let input = "\
struct S { foo_long: u8, bar: u32 }
union U { foo_long: u8, bar: u32 }
enum E { FooLong, Bar }
enum V { FooThing { foo_long: u8, bar: u32 }, Bar }
";
    let want = "\
enum E { Bar, FooLong }
enum V { Bar, FooThing { bar: u32, foo_long: u8 } }
struct S { bar: u32, foo_long: u8 }
union U { bar: u32, foo_long: u8 }
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, want);
}

#[test]
fn single_line_reorders_struct_init_and_keeps_rest_last_by_default() {
    let input = "\
struct S { bar: u32, foo_long: u8, foo: bool }

fn make(base: S, foo_long: u8, bar: u32, foo: bool) -> S {
    S { foo_long, bar, foo, ..base }
}
";
    let want = "\
struct S { bar: u32, foo: bool, foo_long: u8 }

fn make(base: S, foo_long: u8, bar: u32, foo: bool) -> S {
    S { bar, foo, foo_long, ..base }
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, want);
}

#[test]
fn fn_args_keep_source_order_by_default() {
    let input = "\
fn make(foo_long: u8, bar: u32, foo: bool) {}

trait T {
    fn make(foo_long: u8, bar: u32, foo: bool);
}

impl T for S {
    fn make(foo_long: u8, bar: u32, foo: bool) {}
}
";
    let want = "\
trait T {
    fn make(foo_long: u8, bar: u32, foo: bool);
}

impl T for S {
    fn make(foo_long: u8, bar: u32, foo: bool) {}
}
fn make(foo_long: u8, bar: u32, foo: bool) {}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, want);
}

#[test]
fn fn_args_reorders_single_line_params() {
    let input = "\
fn make(foo_long: u8, bar: u32, foo: bool) {}

trait T {
    fn make(foo_long: u8, bar: u32, foo: bool);
}

impl T for S {
    fn make(foo_long: u8, bar: u32, foo: bool) {}
}
";
    let want = "\
trait T {
    fn make(bar: u32, foo: bool, foo_long: u8);
}

impl T for S {
    fn make(bar: u32, foo: bool, foo_long: u8) {}
}
fn make(bar: u32, foo: bool, foo_long: u8) {}
";
    let out = reorder_source_with(input, &fn_args_on()).unwrap();
    assert_eq!(out, want);
}

#[test]
fn fn_args_reorders_multiline_params_and_keeps_receiver_first() {
    let input = "\
impl S {
    fn make(
        &mut self,
        foo_long: u8,
        bar: u32,
        foo: bool,
    ) {}
}
";
    let want = "\
impl S {
    fn make(
        &mut self,
        bar: u32,
        foo: bool,
        foo_long: u8,
    ) {}
}
";
    let out = reorder_source_with(input, &fn_args_on()).unwrap();
    assert_eq!(out, want);
}

#[test]
fn fn_args_preserves_existing_multiline_param_blanks() {
    let input = "\
fn make(
    foo: bool,

    foo_long: u8,
    bar: u32,
) {}
";
    let want = "\
fn make(
    bar: u32,
    foo: bool,

    foo_long: u8,
) {}
";
    let out = reorder_source_with(input, &fn_args_on()).unwrap();
    assert_eq!(out, want);
}

#[test]
fn no_fields_disables_single_line_reorder() {
    let input = "\
struct S { foo_long: u8, bar: u32 }

fn make(foo_long: u8, bar: u32) -> S {
    S { foo_long: 1, bar: 2 }
}
";
    let cfg = Config {
        no_fields: true,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    assert_eq!(out, input);
}
