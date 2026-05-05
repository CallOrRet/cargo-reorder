//! End-to-end coverage of the 16 top-level item categories. One test per
//! category checks: (a) the item is parsed and emitted intact and
//! (b) it lands in the expected slot relative to its neighbours.

use cargo_reorder::reorder_source;

#[test]
fn extern_crate_lands_first() {
    let input = "use std::fmt;\nextern crate alloc;\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("extern crate alloc").unwrap() < out.find("use std::fmt").unwrap(),
        "{out}"
    );
}

#[test]
fn const_static_type_relative_order() {
    let input = "type Map = u32;\nstatic S: u32 = 0;\nconst C: u32 = 1;\n";
    let out = reorder_source(input).unwrap();
    let p_const = out.find("const C").unwrap();
    let p_static = out.find("static S").unwrap();
    let p_type = out.find("type Map").unwrap();
    assert!(
        p_const < p_static && p_static < p_type,
        "const < static < type:\n{out}"
    );
}

#[test]
fn trait_then_enum_then_struct_with_default_flags() {
    // Default is trait-first (Trait weight 49, Enum 50, Struct 51).
    let input = "struct S;\nenum E { A }\ntrait T {}\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("trait T").unwrap() < out.find("enum E").unwrap()
            && out.find("enum E").unwrap() < out.find("struct S").unwrap(),
        "{out}"
    );
}

#[test]
fn union_groups_with_struct() {
    let input = "fn helper() {}\nunion U { x: u32, y: f32 }\nstruct S;\n";
    let out = reorder_source(input).unwrap();
    // Both at Struct weight (51); preserve their source order.
    let p_union = out.find("union U").unwrap();
    let p_struct = out.find("struct S").unwrap();
    let p_fn = out.find("fn helper").unwrap();
    assert!(
        p_union < p_fn && p_struct < p_fn,
        "union/struct before fn:\n{out}"
    );
}

#[test]
fn trait_alias_treated_as_trait() {
    // `trait Foo = Bar + Baz;` is an unstable feature but still parsed by
    // syn as `Item::TraitAlias`. It should classify as Trait.
    let input = "fn helper() {}\ntrait MyAlias = Send + Sync;\nstruct S;\n";
    let out = reorder_source(input).unwrap();
    let p_struct = out.find("struct S").unwrap();
    let p_alias = out.find("trait MyAlias").unwrap();
    let p_fn = out.find("fn helper").unwrap();
    // trait-first default: trait alias < struct < fn.
    assert!(
        p_alias < p_struct && p_struct < p_fn,
        "trait_alias < struct < fn:\n{out}"
    );
}

#[test]
fn fn_then_async_fn() {
    let input = "async fn b() {}\nfn a() {}\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("fn a").unwrap() < out.find("async fn b").unwrap(),
        "{out}"
    );
}

#[test]
fn unsafe_fn_classifies_as_fn() {
    let input = "async fn b() {}\nunsafe fn dangerous() {}\nstruct S;\n";
    let out = reorder_source(input).unwrap();
    let p_struct = out.find("struct S").unwrap();
    let p_unsafe = out.find("unsafe fn dangerous").unwrap();
    let p_async = out.find("async fn b").unwrap();
    assert!(
        p_struct < p_unsafe && p_unsafe < p_async,
        "struct < unsafe fn < async fn:\n{out}"
    );
}

#[test]
fn const_fn_classifies_as_fn() {
    let input = "async fn b() {}\nconst fn c() -> u32 { 1 }\nstruct S;\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("struct S").unwrap() < out.find("const fn c").unwrap(),
        "{out}"
    );
    assert!(
        out.find("const fn c").unwrap() < out.find("async fn b").unwrap(),
        "{out}"
    );
}

#[test]
fn extern_block_with_non_c_abi_classified_same() {
    // `extern "Rust" {}` and `extern "system" {}` are both ForeignMod
    // (Item::ForeignMod), should sort identically to extern "C".
    let input = "\
fn helper() {}
extern \"Rust\" {
    fn rust_fn();
}
extern \"system\" {
    fn sys_fn();
}
struct S;
";
    let out = reorder_source(input).unwrap();
    let p_struct = out.find("struct S").unwrap();
    let p_rust = out.find("extern \"Rust\"").unwrap();
    let p_sys = out.find("extern \"system\"").unwrap();
    let p_fn = out.find("fn helper").unwrap();
    assert!(
        p_struct < p_rust && p_rust < p_sys && p_sys < p_fn,
        "all extern blocks classify the same:\n{out}"
    );
}

#[test]
fn cfg_test_on_non_mod_does_not_promote_to_test_mod() {
    // `#[cfg(test)] fn helper()` is just a cfg-gated fn — not a test
    // mod. It should NOT be forced to file end; it's a regular Fn.
    let input = "\
#[cfg(test)]
fn test_only_helper() {}

fn always() {}
";
    let out = reorder_source(input).unwrap();
    // Both fns same category; preserve source order.
    let p_helper = out.find("fn test_only_helper").unwrap();
    let p_always = out.find("fn always").unwrap();
    assert!(
        p_helper < p_always,
        "#[cfg(test)] on a fn must not be treated as TestMod:\n{out}"
    );
}

#[test]
fn multiple_test_mods_all_at_end() {
    let input = "\
fn x() {}

#[cfg(test)]
mod test_one { #[test] fn a() {} }

fn y() {}

#[cfg(test)]
mod test_two { #[test] fn b() {} }
";
    let out = reorder_source(input).unwrap();
    let p_x = out.find("fn x").unwrap();
    let p_y = out.find("fn y").unwrap();
    let p_t1 = out.find("mod test_one").unwrap();
    let p_t2 = out.find("mod test_two").unwrap();
    assert!(
        p_x < p_y && p_y < p_t1 && p_t1 < p_t2,
        "all #[cfg(test)] mods sorted to end, in source order:\n{out}"
    );
}

#[test]
fn extern_block_lands_before_fn() {
    let input = "fn helper() {}\nextern \"C\" {\n    fn c_lib();\n}\nstruct S;\n";
    let out = reorder_source(input).unwrap();
    let p_struct = out.find("struct S").unwrap();
    let p_extern = out.find("extern \"C\"").unwrap();
    let p_fn = out.find("fn helper").unwrap();
    assert!(
        p_struct < p_extern && p_extern < p_fn,
        "struct < extern{{}} < fn:\n{out}"
    );
}

#[test]
fn macro_rules_with_no_callers_sorts_to_end() {
    let input = "macro_rules! unused { () => {}; }\nfn z() {}\nstruct S;\n";
    let out = reorder_source(input).unwrap();
    let p_struct = out.find("struct S").unwrap();
    let p_fn = out.find("fn z").unwrap();
    let p_macro = out.find("macro_rules! unused").unwrap();
    assert!(
        p_struct < p_fn && p_fn < p_macro,
        "struct < fn < unused-macro:\n{out}"
    );
}

#[test]
fn cfg_test_mod_forced_to_end() {
    let input = "#[cfg(test)]\nmod tests { fn t() {} }\nfn other() {}\nstruct S;\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("fn other").unwrap() < out.find("mod tests").unwrap(),
        "{out}"
    );
    assert!(
        out.find("struct S").unwrap() < out.find("fn other").unwrap(),
        "{out}"
    );
}

#[test]
fn full_canonical_pipeline() {
    // One file with every item kind in the wrong order.
    let input = r#"
#[cfg(test)]
mod tests { #[test] fn t() {} }

async fn ay() {}
fn z() {}
macro_rules! m { () => {}; }
extern "C" { fn ext(); }
trait T {}
struct S;
enum E { A, B }
type Alias = u32;
static SVAR: u32 = 0;
const KAY: u32 = 1;
mod child;
pub use crate::reexport::Foo;
use serde::Serialize;
extern crate alloc;
"#;
    let out = reorder_source(input).unwrap();
    let want = [
        "extern crate alloc",
        "use serde::Serialize",
        "pub use crate::reexport::Foo",
        // `macro_rules! m` is pulled before `mod child;` by the
        // conservative cross-file constraint (no source_path is given,
        // so we assume the child file might use the macro).
        "macro_rules! m",
        "mod child;",
        "const KAY",
        "static SVAR",
        "type Alias",
        // trait-first default: Trait (49) < Enum (50) < Struct (51).
        "trait T",
        "enum E",
        "struct S",
        "extern \"C\"",
        "fn z()",
        "async fn ay",
        "#[cfg(test)]",
    ];
    let mut last = 0;
    for needle in want {
        let pos = out
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle}:\n{out}"));
        assert!(pos >= last, "expected {needle} after pos {last}\n{out}");
        last = pos;
    }
}
