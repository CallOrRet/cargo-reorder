//! `macro_rules!` is textually scoped: a macro is visible only AFTER its
//! definition. The reorderer therefore:
//!
//! * sorts `macro_rules!` definitions by their normal `Category::Macro`
//!   weight (near the end of the file) by default, then
//! * post-processes the sorted output: for every `macro_rules! foo`, if
//!   any earlier-sorted item invokes `foo!`, the definition is yanked
//!   back to land just before that first caller.
//!
//! Bare top-level macro invocations (`Item::Macro` with no `ident`, e.g.
//! `lazy_static! { ... }`) are use-sites whose expansion produces items
//! at exactly that location, so they pin in place as barriers — no other
//! item is reordered across them.

use cargo_reorder::reorder_source;

#[test]
fn macro_rules_pulled_to_just_before_its_caller() {
    // Real shape from serde_core/src/ser/fmt.rs: a macro_rules! defined
    // between two impls and invoked by the second one. The macro must
    // remain above its caller in the output.
    let input = "\
impl Foo {
    fn first(&self) {}
}

macro_rules! make_methods {
    () => { fn from_macro(&self) {} };
}

impl Bar {
    make_methods!();
}
";
    let out = reorder_source(input).unwrap();
    let p_macro = out.find("macro_rules! make_methods").unwrap();
    let p_caller = out.find("impl Bar").unwrap();
    assert!(
        p_macro < p_caller,
        "macro_rules! must precede its caller after reorder:\n{out}"
    );
}

#[test]
fn unused_macro_rules_sorts_to_end_by_default() {
    // No item invokes `unused!`, so it should fall in `Category::Macro`
    // near the bottom of the file.
    let input = "\
macro_rules! unused { () => {}; }
fn z() {}
const A: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    let p_const = out.find("const A").unwrap();
    let p_fn = out.find("fn z()").unwrap();
    let p_mac = out.find("macro_rules! unused").unwrap();
    assert!(p_const < p_fn, "{out}");
    assert!(p_fn < p_mac, "unused macro should sort to the end:\n{out}");
}

#[test]
fn macro_rules_caller_inside_function_body_still_constrains() {
    let input = "\
fn use_it() {
    say!(\"hi\");
}

macro_rules! say {
    ($e:expr) => { let _ = $e; };
}
";
    let out = reorder_source(input).unwrap();
    let p_macro = out.find("macro_rules! say").unwrap();
    let p_caller = out.find("fn use_it").unwrap();
    assert!(
        p_macro < p_caller,
        "macro must precede caller even when call is inside a fn body:\n{out}"
    );
}

#[test]
fn chain_of_macros_both_pulled_above_caller() {
    // `fn user` bare-calls `m_a`, which in turn expands to `m_b!()`.
    // Both macro definitions must precede `fn user` (Rust resolves
    // `m_b!()` at user's call site, when m_a expands). Their relative
    // order to each other is unconstrained — Rust does not require
    // m_b's definition to come before m_a's, only that both are
    // textually visible at user.
    let input = "\
fn user() { m_a!(); }

macro_rules! m_a {
    () => { m_b!() };
}

macro_rules! m_b {
    () => { 1 + 1 };
}
";
    let out = reorder_source(input).unwrap();
    let p_a = out.find("macro_rules! m_a").unwrap();
    let p_b = out.find("macro_rules! m_b").unwrap();
    let p_user = out.find("fn user").unwrap();
    assert!(
        p_a < p_user && p_b < p_user,
        "both macros must precede caller:\n{out}"
    );
}

#[test]
fn mutually_recursive_macros_dont_loop_forever() {
    // a calls b, b calls a. Rust accepts this (name resolution is
    // order-independent for items), but our post-pass used to
    // oscillate trying to satisfy a < b AND b < a. After the
    // transitive-closure refactor, the macro→macro edge is no longer
    // a constraint; only the user's call site is.
    let input = "\
fn user() { a!(); }

macro_rules! a { () => { b!() }; }
macro_rules! b { () => { a!() }; }
";
    let out = reorder_source(input).unwrap();
    let p_a = out.find("macro_rules! a").unwrap();
    let p_b = out.find("macro_rules! b").unwrap();
    let p_user = out.find("fn user").unwrap();
    assert!(p_a < p_user && p_b < p_user, "{out}");
    // Idempotent.
    let out2 = reorder_source(&out).unwrap();
    assert_eq!(out, out2, "must converge");
}

#[test]
fn bare_top_level_invocation_is_a_barrier() {
    // `lazy_static!` is `Item::Macro` with `ident = None`. It expands at
    // its location, so no other item is reordered across it.
    let input = "\
fn after() {}

lazy_static! {
    static ref FOO: u32 = compute();
}

fn before() {}
";
    let out = reorder_source(input).unwrap();
    let p_after = out.find("fn after").unwrap();
    let p_lazy = out.find("lazy_static!").unwrap();
    let p_before = out.find("fn before").unwrap();
    assert!(
        p_after < p_lazy && p_lazy < p_before,
        "bare top-level macro invocation must not be crossed:\n{out}"
    );
}

#[test]
fn cfg_gated_alternative_macro_definitions_both_pulled_above_caller() {
    // Real shape from ripgrep's globset crate: two `macro_rules! debug`
    // definitions guarded by mutually-exclusive cfg attributes. Both must
    // end up above any caller, in their original source order.
    let input = "\
fn caller() { debug!(\"hi\"); }

#[cfg(not(feature = \"log\"))]
macro_rules! debug { ($($t:tt)*) => {}; }

#[cfg(feature = \"log\")]
macro_rules! debug { ($($t:tt)*) => { ::log::debug!($($t)*); }; }
";
    let out = reorder_source(input).unwrap();
    let p_first = out.find("not(feature = \"log\")").unwrap();
    let p_second = out.find("#[cfg(feature = \"log\")]").unwrap();
    let p_caller = out.find("fn caller").unwrap();
    assert!(
        p_first < p_second,
        "preserve source order between same-name defs:\n{out}"
    );
    assert!(
        p_second < p_caller,
        "all definitions must precede the caller:\n{out}"
    );

    // Idempotence: a second reorder must be a no-op.
    let out2 = reorder_source(&out).unwrap();
    assert_eq!(out, out2, "macro fix-up must be idempotent");
}

#[test]
fn macro_fixup_is_deterministic_across_runs() {
    // Several macros all referenced by one caller — exercises the
    // fix-up's convergence loop. Two separate runs must produce the
    // same output (HashMap iteration order is otherwise non-stable).
    let input = "\
fn user() { a!(); b!(); c!(); }

macro_rules! c { () => {}; }
macro_rules! b { () => {}; }
macro_rules! a { () => {}; }
";
    let r1 = reorder_source(input).unwrap();
    let r2 = reorder_source(&r1).unwrap();
    assert_eq!(r1, r2, "must be idempotent");

    // All three macros end up before fn user.
    let p_user = r1.find("fn user").unwrap();
    for name in ["macro_rules! a", "macro_rules! b", "macro_rules! c"] {
        let p = r1.find(name).unwrap();
        assert!(p < p_user, "{name} must precede caller:\n{r1}");
    }
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
