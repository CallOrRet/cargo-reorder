//! Macro placement tests. Every `macro_rules!` (with or without ident),
//! every bare top-level macro invocation, and every `#[macro_use] mod`
//! is treated as a hard barrier: it stays in its source position and no
//! other item is reordered across it. The trade-off: simple, always
//! correct, sometimes leaves more in original order than strictly
//! necessary.

use cargo_reorder::reorder_source;

#[test]
fn macro_rules_pins_in_place() {
    // The macro is between two functions in source order. Under
    // barrier semantics it stays exactly where it was; `fn before`
    // and `fn after` keep their relative position around it (each is
    // alone in its own segment, so within-segment sort is trivial).
    let input = "\
fn before() {}

macro_rules! m { () => {}; }

fn after() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let p_before = out.find("fn before").unwrap();
    let p_macro = out.find("macro_rules! m").unwrap();
    let p_after = out.find("fn after").unwrap();
    assert!(
        p_before < p_macro && p_macro < p_after,
        "macro must stay in source position:\n{out}"
    );
}

#[test]
fn macro_use_mod_is_a_barrier() {
    // `#[macro_use] mod foo;` exports macros from foo into the parent
    // scope. Items declared after it (and child mods declared after
    // it) may rely on those macros being visible. Pinning the
    // `#[macro_use] mod` preserves that.
    let input = "\
mod alpha;

#[macro_use]
mod helpers;

mod beta;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let p_alpha = out.find("mod alpha").unwrap();
    let p_helpers = out.find("mod helpers").unwrap();
    let p_beta = out.find("mod beta").unwrap();
    assert!(
        p_alpha < p_helpers && p_helpers < p_beta,
        "#[macro_use] mod must stay between its source neighbours:\n{out}"
    );
}

#[test]
fn plain_mod_is_not_a_barrier() {
    // A non-macro_use `mod foo;` is a regular item — it sorts by
    // category like everything else.
    let input = "\
fn z() {}
mod inner;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // Default order: `mod` (weight 10) before `fn` (weight 90).
    let p_mod = out.find("mod inner").unwrap();
    let p_fn = out.find("fn z").unwrap();
    assert!(p_mod < p_fn, "plain mod must sort by category:\n{out}");
}

#[test]
fn no_macros_means_normal_sort() {
    let input = "fn b() {}\nconst A: u32 = 1;\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("const A").unwrap() < out.find("fn b").unwrap(),
        "{out}"
    );
}
#[test]
fn many_macros_does_not_panic_or_loop() {
    // Sanity: a file with several macros sprinkled between items
    // doesn't break sorting or recursion. Each macro is its own
    // segment; segments are sorted lexicographically by (segment,
    // category) — which here is degenerate (1 item per segment) but
    // must still terminate cleanly and be idempotent.
    let input = "\
fn a() {}
macro_rules! m1 { () => {}; }
fn b() {}
macro_rules! m2 { () => {}; }
fn c() {}
macro_rules! m3 { () => {}; }
fn d() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let again = reorder_source(&out).unwrap();
    assert_eq!(out, again, "idempotent");
    // All four functions stay in source order.
    let pa = out.find("fn a").unwrap();
    let pb = out.find("fn b").unwrap();
    let pc = out.find("fn c").unwrap();
    let pd = out.find("fn d").unwrap();
    assert!(
        pa < pb && pb < pc && pc < pd,
        "fn relative order preserved across barriers:\n{out}"
    );
}

#[test]
fn nothing_reorders_across_macro_rules() {
    // `const A` and `use std::fmt;` would normally sort before
    // `struct B` (use(31) < const(40) < struct(51) under the default
    // mod-first weights), but the macro between use and the others
    // pins everything: items above the macro can't move below, items
    // below can't move above.
    let input = "\
struct B;

macro_rules! m { () => {}; }

use std::fmt;
const A: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // Above the barrier: only `struct B` (alone). Below: use + const,
    // sorted by category (use first, then const).
    let p_struct = out.find("struct B").unwrap();
    let p_macro = out.find("macro_rules!").unwrap();
    let p_use = out.find("use std::fmt").unwrap();
    let p_const = out.find("const A").unwrap();
    assert!(p_struct < p_macro, "struct must stay above macro:\n{out}");
    assert!(p_macro < p_use, "use must stay below macro:\n{out}");
    assert!(
        p_use < p_const,
        "below-barrier section sorts by category:\n{out}"
    );
}

#[test]
fn bare_top_level_invocation_is_a_barrier() {
    // `lazy_static! { ... }` is `Item::Macro` with `ident = None`. It
    // expands at its location, so no other item is reordered across
    // it. Same shape as a `macro_rules!` barrier.
    let input = "\
fn after() {}

lazy_static! {
    static ref FOO: u32 = compute();
}

fn before() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let p_after = out.find("fn after").unwrap();
    let p_lazy = out.find("lazy_static!").unwrap();
    let p_before = out.find("fn before").unwrap();
    assert!(
        p_after < p_lazy && p_lazy < p_before,
        "bare top-level macro invocation must not be crossed:\n{out}"
    );
}

#[test]
fn cfg_gated_macros_each_pin_independently() {
    // Two `macro_rules! debug` definitions guarded by mutually-
    // exclusive cfg attrs. Each is a separate barrier; their
    // relative source order is preserved verbatim.
    let input = "\
#[cfg(not(feature = \"log\"))]
macro_rules! debug { () => {}; }

#[cfg(feature = \"log\")]
macro_rules! debug { () => {}; }

fn user() { debug!(); }
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let p_first = out.find("not(feature = \"log\")").unwrap();
    let p_second = out.find("#[cfg(feature = \"log\")]").unwrap();
    let p_user = out.find("fn user").unwrap();
    assert!(p_first < p_second, "preserve source order:\n{out}");
    assert!(
        p_second < p_user,
        "user must stay below both barriers:\n{out}"
    );
}

