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

#[test]
fn macro_use_mod_pins_as_barrier_under_pub_mod_first() {
    // Models the failure pattern observed on syn/src/lib.rs: a
    // `#[macro_use] mod foo;` defines (transitively) macros that a later
    // `pub mod bar;` invokes textually. With `--pub-mod-first`, the naive
    // sort would lift `pub mod bar;` ahead of `mod foo;`, making the
    // macro unreachable. The barrier semantics keep them on opposite
    // sides of the original split.
    let input = "\
use std::fmt;

#[macro_use]
mod helpers;

pub mod consumer;
mod private_helper;
";
    let cfg = cargo_reorder::Config {
        pub_mod_first: true,
        ..Default::default()
    };
    let out = cargo_reorder::reorder_source_with(input, &cfg).unwrap();
    let p_use = out.find("#[macro_use]").unwrap();
    let p_consumer = out.find("pub mod consumer").unwrap();
    assert!(
        p_use < p_consumer,
        "`#[macro_use] mod` must stay before consuming `pub mod`:\n{out}"
    );
}

#[test]
fn macro_use_mod_barrier_partitions_segments() {
    // Items on opposite sides of a `#[macro_use] mod` cannot cross it,
    // even when sort weight would otherwise put them together. Here the
    // `use` after the barrier must NOT be lifted above it.
    let input = "\
mod a;

#[macro_use]
mod helpers;

use std::fmt;
";
    let out = reorder_source(input).unwrap();
    let p_barrier = out.find("#[macro_use]").unwrap();
    let p_use = out.find("use std::fmt").unwrap();
    assert!(
        p_barrier < p_use,
        "barrier must stay above the `use` that originally followed it:\n{out}"
    );
}

#[test]
fn use_of_local_macro_pulls_macro_above_it() {
    // Real shape from xh's src/formatting/palette.rs: a `pub(crate) use
    // NAME;` re-exports a `macro_rules! NAME` defined in the same
    // module. `macro_rules!` macros without `#[macro_export]` are
    // textually scoped, so the `use` only resolves if the macro is
    // defined above it. Default category sort would push `macro_rules!`
    // toward the end of the file (Category::Macro weight) and the
    // re-export to the top — breaking the build. The yank pipeline
    // must treat the `use` as a caller and pull the macro above it.
    let input = "\
//! re-exports a local macro

macro_rules! palette {
    () => {};
}

pub(crate) use palette;

pub(crate) mod util {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let macro_pos = out.find("macro_rules! palette").unwrap();
    let use_pos = out.find("pub(crate) use palette;").unwrap();
    assert!(
        macro_pos < use_pos,
        "macro_rules! must stay textually above the re-export:\n{out}"
    );
    // And the second pass is stable.
    let again = reorder_source(&out).unwrap();
    assert_eq!(out, again, "not idempotent:\n{out}\nvs\n{again}");
}

#[test]
fn renamed_use_of_local_macro_also_pulls_macro_above() {
    // `pub(crate) use foo as bar;` still references `foo` — the macro
    // must come above. The renamed name `bar` is irrelevant.
    let input = "\
macro_rules! foo {
    () => {};
}

pub(crate) use foo as bar;

fn z() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let macro_pos = out.find("macro_rules! foo").unwrap();
    let use_pos = out.find("use foo as bar").unwrap();
    assert!(
        macro_pos < use_pos,
        "renamed re-export must still come after the macro:\n{out}"
    );
}

#[test]
fn use_grouped_brace_with_local_macro_keeps_order() {
    // `pub use {alpha, beta};` where `alpha` is a local macro and
    // `beta` is just a plain item — the macro must still come first.
    let input = "\
macro_rules! alpha {
    () => {};
}

struct Beta;

pub(crate) use {alpha, Beta};

fn z() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let macro_pos = out.find("macro_rules! alpha").unwrap();
    let use_pos = out
        .find("use {alpha")
        .unwrap_or_else(|| out.find("use { alpha").expect("either spacing must appear"));
    assert!(
        macro_pos < use_pos,
        "grouped use referencing a local macro must come after the macro:\n{out}"
    );
}

#[test]
fn macro_use_mod_stays_above_external_mod_siblings_no_path() {
    // Path-less call (`reorder_source` has no mod_dir), so the
    // recursive sibling scan can't open child files and falls back
    // to the conservative "needed=true" path. The end result is the
    // same: `#[macro_use] mod tree;` stays above `mod parser;`.
    let input = "\
type Result<'a, T = ()> = std::result::Result<T, Error<'a>>;
const RECURSION_LIMIT: usize = 256;

#[cfg(test)]
#[macro_use]
pub mod tree;

mod alpha;
mod beta;
mod parser;
mod zulu;
";
    let out = cargo_reorder::reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // The barrier semantics force `#[macro_use] mod tree;` to keep
    // its place relative to all sibling external mods. In particular
    // it must NOT land below `mod parser;`.
    let tree_pos = out.find("pub mod tree;").unwrap();
    let parser_pos = out.find("mod parser;").unwrap();
    assert!(
        tree_pos < parser_pos,
        "#[macro_use] mod must stay above sibling external mods (might call its macros):\n{out}"
    );
    let again = cargo_reorder::reorder_source(&out).unwrap();
    assert_eq!(out, again, "not idempotent");
}

#[test]
fn macro_use_mod_recursive_scan_finds_call_in_child_file() {
    // Real-FS variant: lib.rs has `#[macro_use] mod tree;` and a
    // sibling `mod parser;`. The call to `tree!()` lives inside
    // src/parser.rs (depth 1 of the recursion). With a real path
    // passed in, `compute_macro_use_relevance` opens parser.rs,
    // runs `find_calls`, finds `tree!()`, and keeps the barrier.
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static UNIQ: AtomicU64 = AtomicU64::new(0);
    let mut td: PathBuf = env::temp_dir();
    let n = UNIQ.fetch_add(1, Ordering::Relaxed);
    td.push(format!(
        "cargo-reorder-macros-recurse-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&td);
    fs::create_dir_all(&td).unwrap();
    let src_dir = td.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let lib = src_dir.join("lib.rs");
    fs::write(
        &lib,
        "\
type Result<'a, T = ()> = std::result::Result<T, ()>;
const RECURSION_LIMIT: usize = 256;

#[macro_use]
pub mod tree;

mod alpha;
mod parser;
mod zulu;
",
    )
    .unwrap();
    fs::write(src_dir.join("tree.rs"), "macro_rules! tree { () => {}; }\n").unwrap();
    fs::write(src_dir.join("alpha.rs"), "pub fn alpha() {}\n").unwrap();
    // The call lives at the top level of parser.rs — the recursive
    // scan opens this file at depth 1 and finds it.
    fs::write(src_dir.join("parser.rs"), "fn parse() { tree!(); }\n").unwrap();
    fs::write(src_dir.join("zulu.rs"), "pub fn zulu() {}\n").unwrap();

    let input = fs::read_to_string(&lib).unwrap();
    let cfg = cargo_reorder::Config::default();
    let out = cargo_reorder::reorder_source_with_path(&input, Some(&lib), &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let tree_pos = out.find("pub mod tree;").unwrap();
    let parser_pos = out.find("mod parser;").unwrap();
    assert!(
        tree_pos < parser_pos,
        "recursive scan must find tree!() in parser.rs and keep the barrier:\n{out}"
    );

    let _ = fs::remove_dir_all(&td);
}

#[test]
fn macro_use_mod_recursive_scan_descends_into_subfolders() {
    // Same as above, but the `tree!()` call lives one level deeper:
    // src/parser.rs declares `mod helpers;` and the call is in
    // src/parser/helpers.rs. The recursive scan should descend
    // (depth 2) and still find it.
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static UNIQ: AtomicU64 = AtomicU64::new(0);
    let mut td: PathBuf = env::temp_dir();
    let n = UNIQ.fetch_add(1, Ordering::Relaxed);
    td.push(format!(
        "cargo-reorder-macros-recurse-deep-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&td);
    fs::create_dir_all(&td).unwrap();
    let src = td.join("src");
    let parser_dir = src.join("parser");
    fs::create_dir_all(&parser_dir).unwrap();

    let lib = src.join("lib.rs");
    fs::write(
        &lib,
        "\
#[macro_use]
pub mod tree;

mod parser;
",
    )
    .unwrap();
    fs::write(src.join("tree.rs"), "macro_rules! tree { () => {}; }\n").unwrap();
    fs::write(src.join("parser.rs"), "pub mod helpers;\n").unwrap();
    fs::write(parser_dir.join("helpers.rs"), "fn h() { tree!(); }\n").unwrap();

    let input = fs::read_to_string(&lib).unwrap();
    let cfg = cargo_reorder::Config::default();
    let out = cargo_reorder::reorder_source_with_path(&input, Some(&lib), &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let tree_pos = out.find("pub mod tree;").unwrap();
    let parser_pos = out.find("mod parser;").unwrap();
    assert!(
        tree_pos < parser_pos,
        "depth-2 recursion must still find tree!() in parser/helpers.rs:\n{out}"
    );

    let _ = fs::remove_dir_all(&td);
}

#[test]
fn macro_use_mod_recursive_scan_can_demote_when_no_call_anywhere() {
    // The recursive scan also gives us the *positive* side: when no
    // sibling external mod actually calls the exported macro at any
    // depth, the barrier can be demoted and the `#[macro_use] mod`
    // sorts by normal category weight. Output must still parse and
    // be idempotent.
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static UNIQ: AtomicU64 = AtomicU64::new(0);
    let mut td: PathBuf = env::temp_dir();
    let n = UNIQ.fetch_add(1, Ordering::Relaxed);
    td.push(format!(
        "cargo-reorder-macros-recurse-demote-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&td);
    fs::create_dir_all(&td).unwrap();
    let src = td.join("src");
    fs::create_dir_all(&src).unwrap();

    let lib = src.join("lib.rs");
    fs::write(
        &lib,
        "\
#[macro_use]
pub mod tree;

mod alpha;
mod beta;
",
    )
    .unwrap();
    fs::write(
        src.join("tree.rs"),
        "macro_rules! never_called { () => {}; }\n",
    )
    .unwrap();
    fs::write(src.join("alpha.rs"), "pub fn a() {}\n").unwrap();
    fs::write(src.join("beta.rs"), "pub fn b() {}\n").unwrap();

    let input = fs::read_to_string(&lib).unwrap();
    let cfg = cargo_reorder::Config::default();
    let out = cargo_reorder::reorder_source_with_path(&input, Some(&lib), &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let again = cargo_reorder::reorder_source_with_path(&out, Some(&lib), &cfg).unwrap();
    assert_eq!(out, again, "not idempotent");

    let _ = fs::remove_dir_all(&td);
}

#[test]
fn macro_use_mod_can_still_demote_when_only_inline_siblings_dont_call() {
    // The optimization still fires when every sibling is either a
    // non-mod or an *inline* mod we can fully scan, and none of them
    // actually invoke the exported macro. In that case the barrier is
    // safe to demote and the `#[macro_use] mod` participates in
    // normal category sorting.
    let input = "\
fn earlier() {}

#[macro_use]
mod helpers {
    macro_rules! never_used {
        () => {};
    }
}

mod inline_sibling {
    pub fn ok() {}
}

fn later() {}
";
    let out = cargo_reorder::reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // The two functions, the macro_use mod, and the inline sibling
    // can all be sorted by normal category weight. The key check is
    // simply that the output parses and is idempotent — we don't
    // assert a specific order because the demotion is an internal
    // optimization, not a contract.
    let again = cargo_reorder::reorder_source(&out).unwrap();
    assert_eq!(out, again, "not idempotent");
}

#[test]
fn macro_use_recursive_scan_dedupes_same_file_via_path_alias() {
    // Two siblings declare `#[path = "shared.rs"] mod a;` and
    // `#[path = "shared.rs"] mod b;` — both resolve to the same
    // physical file. Without canonical-path de-dup the recursive
    // scan would open and parse it twice; with de-dup it parses
    // once. Either way the result must be the same and the run
    // must not loop or crash.
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static UNIQ: AtomicU64 = AtomicU64::new(0);
    let mut td: PathBuf = env::temp_dir();
    let n = UNIQ.fetch_add(1, Ordering::Relaxed);
    td.push(format!(
        "cargo-reorder-macros-dedupe-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&td);
    fs::create_dir_all(&td).unwrap();
    let src = td.join("src");
    fs::create_dir_all(&src).unwrap();

    let lib = src.join("lib.rs");
    fs::write(
        &lib,
        "\
#[macro_use]
pub mod tree;

#[path = \"shared.rs\"]
mod a;

#[path = \"shared.rs\"]
mod b;
",
    )
    .unwrap();
    fs::write(src.join("tree.rs"), "macro_rules! tree { () => {}; }\n").unwrap();
    fs::write(src.join("shared.rs"), "pub fn shared() {}\n").unwrap();

    let input = fs::read_to_string(&lib).unwrap();
    let cfg = cargo_reorder::Config::default();
    let out = cargo_reorder::reorder_source_with_path(&input, Some(&lib), &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let again = cargo_reorder::reorder_source_with_path(&out, Some(&lib), &cfg).unwrap();
    assert_eq!(out, again, "not idempotent");

    let _ = fs::remove_dir_all(&td);
}

#[test]
fn macro_use_recursive_scan_breaks_path_cycles() {
    // Pathological: a.rs declares `#[path = "b.rs"] mod b_alias;` and
    // b.rs declares `#[path = "a.rs"] mod a_alias;` — a manual
    // `#[path]` cycle. Without dedup the scan would walk a -> b -> a
    // -> b -> ... until depth-cap. With dedup it terminates after
    // visiting each file once. We don't assert internal counters;
    // the invariant we test is just "this completes successfully and
    // is idempotent", i.e. cycle handling doesn't break correctness.
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static UNIQ: AtomicU64 = AtomicU64::new(0);
    let mut td: PathBuf = env::temp_dir();
    let n = UNIQ.fetch_add(1, Ordering::Relaxed);
    td.push(format!(
        "cargo-reorder-macros-cycle-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&td);
    fs::create_dir_all(&td).unwrap();
    let src = td.join("src");
    fs::create_dir_all(&src).unwrap();

    let lib = src.join("lib.rs");
    fs::write(
        &lib,
        "\
#[macro_use]
pub mod tree;

mod a;
",
    )
    .unwrap();
    fs::write(src.join("tree.rs"), "macro_rules! tree { () => {}; }\n").unwrap();
    fs::write(src.join("a.rs"), "#[path = \"b.rs\"]\npub mod b_alias;\n").unwrap();
    fs::write(src.join("b.rs"), "#[path = \"a.rs\"]\npub mod a_alias;\n").unwrap();

    let input = fs::read_to_string(&lib).unwrap();
    let cfg = cargo_reorder::Config::default();
    let out = cargo_reorder::reorder_source_with_path(&input, Some(&lib), &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let again = cargo_reorder::reorder_source_with_path(&out, Some(&lib), &cfg).unwrap();
    assert_eq!(out, again, "not idempotent");

    let _ = fs::remove_dir_all(&td);
}

#[test]
fn macro_call_in_struct_field_position_constrains_def() {
    // Real shape from pest's meta/src/optimizer/restorer.rs: a local
    // `macro_rules! box_tree` is invoked as the value side of a
    // struct-field initialiser, e.g. `Foo { expr: box_tree!(...) }`.
    // The token immediately before `box_tree` is a single `:`, which
    // the path-qualified-call detector used to swallow as if it were
    // `crate::box_tree!(...)`. That made the call site invisible to
    // the yank pipeline, so `macro_rules! box_tree` fell to its
    // normal Category::Macro position near the bottom of the file
    // and ended up below `mod restorer;` — breaking the build,
    // because `mod restorer;` (whose body uses box_tree!) needs the
    // macro defined textually above it.
    let input = "\
mod inner;

macro_rules! make {
    () => { 0 };
}

fn use_in_struct_field() {
    let _ = Foo { value: make!() };
}

struct Foo { value: i32 }
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let macro_pos = out.find("macro_rules! make").unwrap();
    let caller_pos = out.find("fn use_in_struct_field").unwrap();
    assert!(
        macro_pos < caller_pos,
        "macro must be above its caller even when the call is in a `field: name!()` position:\n{out}"
    );
}

#[test]
fn macro_call_in_type_annotation_position_constrains_def() {
    // Same shape, different syntactic position: a local-macro call
    // sits immediately after a single `:` in a `let x: foo!(...) =
    // ...` (rare but legal) or after `:` in a fn-param type. The
    // single colon must NOT be confused with a `::` path separator.
    let input = "\
fn caller() {
    let _: i32 = sum!(1, 2);
}

macro_rules! sum {
    ($a:expr, $b:expr) => { $a + $b };
}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let macro_pos = out.find("macro_rules! sum").unwrap();
    let caller_pos = out.find("fn caller").unwrap();
    assert!(
        macro_pos < caller_pos,
        "macro must be above its caller (single colon != path):\n{out}"
    );
}

#[test]
fn path_qualified_call_still_does_not_constrain_local_macro() {
    // Sanity: path-qualified `crate::name!()` calls really are
    // ignored. A local `macro_rules! name` with no bare callers must
    // still sort to its category position.
    let input = "\
fn caller() {
    crate::name!();
}

macro_rules! name {
    () => {};
}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // Path-qualified call doesn't constrain → macro stays in the
    // Macro bucket (after fn). Caller comes first by category.
    let caller_pos = out.find("fn caller").unwrap();
    let macro_pos = out.find("macro_rules! name").unwrap();
    assert!(
        caller_pos < macro_pos,
        "path-qualified call must not pull macro above caller:\n{out}"
    );
}
