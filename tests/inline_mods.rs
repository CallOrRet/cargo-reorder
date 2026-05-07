//! Coverage for inline-mod reordering. Default ON: bodies are
//! reordered with the same rules, except for the documented skip
//! list (test mods, `#[macro_use]`, pure-`use` mods). With
//! `--no-reorder-inline-mods` (i.e. `Config { no_reorder_inline_mods:
//! true, ..default() }`), inline mod bodies stay byte-identical to
//! the source.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cargo_reorder::{Config, reorder_source, reorder_source_with, reorder_source_with_path};

static UNIQ: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    dir: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let mut dir = env::temp_dir();
        let n = UNIQ.fetch_add(1, Ordering::Relaxed);
        dir.push(format!(
            "cargo-reorder-inline-{name}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn write(&self, rel: &str, src: &str) -> PathBuf {
        let p = self.dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, src).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn pure_use_mod_skipped() {
    // Prelude / __private style: 100% `use` items. Order is part of
    // the contract; do not touch.
    let input = "\
pub mod prelude {
    pub use crate::Foo;
    pub use crate::error::Result;
    pub use crate::traits::*;
}
";
    let out = reorder_source_with(input, &cfg_recurse()).unwrap();
    let body_start = out.find("pub mod prelude {").unwrap();
    let body = &out[body_start..];
    let p_foo = body.find("pub use crate::Foo").unwrap();
    let p_result = body.find("pub use crate::error::Result").unwrap();
    let p_traits = body.find("pub use crate::traits::*").unwrap();
    assert!(
        p_foo < p_result && p_result < p_traits,
        "pure-`use` mod ordering must be preserved:\n{out}"
    );
}

#[test]
fn macro_use_mod_body_skipped() {
    let input = "\
#[macro_use]
mod helpers {
    fn z() {}
    macro_rules! gimme { () => { 1 }; }
    use std::fmt;
}
fn caller() { gimme!(); }
";
    let out = reorder_source_with(input, &cfg_recurse()).unwrap();
    // Body must stay verbatim: `fn z`, then macro_rules, then use.
    let body_start = out.find("mod helpers {").unwrap();
    let body = &out[body_start..];
    let p_z = body.find("fn z()").unwrap();
    let p_macro = body.find("macro_rules! gimme").unwrap();
    let p_use = body.find("use std::fmt").unwrap();
    assert!(
        p_z < p_macro && p_macro < p_use,
        "#[macro_use] mod body must be left verbatim (macro visibility leaks):\n{out}"
    );
}

#[test]
fn idempotent_under_recursion() {
    let input = "\
mod a {
    fn z() {}
    use std::fmt;
    mod inner {
        struct S;
        use core::mem;
        fn helper() {}
    }
    struct Top;
}
fn outer_fn() {}
use serde::Serialize;
";
    let pass1 = reorder_source_with(input, &cfg_recurse()).unwrap();
    let pass2 = reorder_source_with(&pass1, &cfg_recurse()).unwrap();
    assert_eq!(
        pass1, pass2,
        "recursive reorder must be idempotent:\n--- pass1 ---\n{pass1}\n--- pass2 ---\n{pass2}"
    );
}

fn cfg_recurse() -> Config {
    // Inline-mod recursion is the default now; this helper exists only
    // so the test names stay self-documenting.
    Config::default()
}

#[test]
fn cfg_test_mod_body_still_skipped_when_flag_on() {
    // tests_last still pulls the mod to the bottom, but its body must
    // NOT be reshuffled — reordering test fixtures hides intent.
    let input = "\
fn before() {}

#[cfg(test)]
mod tests {
    fn helper_b() {}
    use super::*;
    #[test] fn helper_a() {}
}
";
    let out = reorder_source_with(input, &cfg_recurse()).unwrap();
    // tests mod after `before`.
    assert!(
        out.find("fn before").unwrap() < out.find("mod tests").unwrap(),
        "{out}"
    );
    // Body intact: helper_b first, then `use`, then helper_a — exactly the
    // source order.
    let body_start = out.find("mod tests {").unwrap();
    let body = &out[body_start..];
    let p_b = body.find("fn helper_b").unwrap();
    let p_use = body.find("use super::*").unwrap();
    let p_a = body.find("fn helper_a").unwrap();
    assert!(
        p_b < p_use && p_use < p_a,
        "#[cfg(test)] body must not be reordered even with flag on:\n{out}"
    );
}

#[test]
fn nested_inline_mods_recurse_too() {
    let input = "\
mod outer {
    mod inner {
        fn z() {}
        use core::cmp::Ord;
    }
    fn at_outer() {}
    use std::fmt;
}
";
    let out = reorder_source_with(input, &cfg_recurse()).unwrap();
    let p_outer_use = out.find("use std::fmt").unwrap();
    let p_inner_mod = out.find("mod inner").unwrap();
    let p_outer_fn = out.find("fn at_outer").unwrap();
    // Outer body canonicalised: mod < use < fn (default mod-first +
    // use-after-mods + fn-late).
    assert!(
        p_inner_mod < p_outer_use && p_outer_use < p_outer_fn,
        "outer body order:\n{out}"
    );
    let p_inner_use = out.find("use core::cmp::Ord").unwrap();
    let p_inner_fn = out.find("fn z()").unwrap();
    assert!(
        p_inner_use < p_inner_fn,
        "inner body must also be canonicalised:\n{out}"
    );
}

#[test]
fn flag_preserves_indentation() {
    let input = "\
mod inner {
    fn b() {}
    use std::fmt;
}
";
    let out = reorder_source_with(input, &cfg_recurse()).unwrap();
    assert!(
        out.contains("    use std::fmt;"),
        "use line must keep its 4-space indent:\n{out}"
    );
    assert!(
        out.contains("    fn b() {}"),
        "fn line must keep its 4-space indent:\n{out}"
    );
}

#[test]
fn flag_reorders_inline_mod_body() {
    let input = "\
mod inner {
    fn b() {}
    use std::fmt;
    struct S;
}
";
    let out = reorder_source_with(input, &cfg_recurse()).unwrap();
    let p_use = out.find("use std::fmt").unwrap();
    let p_struct = out.find("struct S").unwrap();
    let p_fn = out.find("fn b()").unwrap();
    assert!(
        p_use < p_struct && p_struct < p_fn,
        "inline body must canonicalise: use < struct < fn:\n{out}"
    );
}

#[test]
fn mod_with_one_non_use_is_eligible() {
    // The "100% `use`" skip rule kicks in only when EVERY item is `use`.
    // A single non-`use` item makes it eligible.
    let input = "\
pub mod almost_prelude {
    fn helper() {}
    pub use crate::Foo;
}
";
    let out = reorder_source_with(input, &cfg_recurse()).unwrap();
    let body_start = out.find("pub mod almost_prelude {").unwrap();
    let body = &out[body_start..];
    let p_use = body.find("pub use crate::Foo").unwrap();
    let p_fn = body.find("fn helper").unwrap();
    assert!(
        p_use < p_fn,
        "with at least one non-use item, body should reorder normally:\n{out}"
    );
}

#[test]
fn flag_off_keeps_test_mod_body_verbatim_too() {
    // Sanity: with flag OFF the cfg(test) skip is moot — we never recurse.
    let input = "\
#[cfg(test)]
mod tests {
    fn helper_b() {}
    use super::*;
}
";
    let out = reorder_source(input).unwrap();
    let body_start = out.find("mod tests {").unwrap();
    let body = &out[body_start..];
    let p_b = body.find("fn helper_b").unwrap();
    let p_use = body.find("use super::*").unwrap();
    assert!(p_b < p_use, "flag off: body verbatim:\n{out}");
}

#[test]
fn multiple_inline_mods_at_same_level() {
    let input = "\
mod a {
    fn z() {}
    use std::a as _;
}
mod b {
    fn z() {}
    use std::b as _;
}
";
    let out = reorder_source_with(input, &cfg_recurse()).unwrap();
    // Each body independently reordered.
    let body_a = &out[out.find("mod a {").unwrap()..out.find("mod b {").unwrap()];
    assert!(
        body_a.find("use std::a").unwrap() < body_a.find("fn z").unwrap(),
        "mod a body reordered:\n{out}"
    );
    let body_b = &out[out.find("mod b {").unwrap()..];
    assert!(
        body_b.find("use std::b").unwrap() < body_b.find("fn z").unwrap(),
        "mod b body reordered:\n{out}"
    );
}

#[test]
fn import_groups_apply_inside_inline_mod() {
    let input = "\
mod inner {
    use crate::a;
    use std::fmt;
    use serde::Serialize;
    fn helper() {}
}
";
    let out = reorder_source_with(input, &cfg_recurse()).unwrap();
    let p_std = out.find("use std::fmt").unwrap();
    let p_ext = out.find("use serde::Serialize").unwrap();
    let p_crate = out.find("use crate::a").unwrap();
    let p_fn = out.find("fn helper").unwrap();
    assert!(
        p_std < p_ext && p_ext < p_crate && p_crate < p_fn,
        "std < external < crate < fn inside inline mod:\n{out}"
    );
}

#[test]
fn opt_out_does_not_touch_inline_mod_body() {
    let input = "\
mod inner {
    fn b() {}
    use std::fmt;
    struct S;
}
";
    let cfg = Config {
        no_reorder_inline_mods: true,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    // With opt-out, only top-level is reordered (one item here, so nothing
    // moves). Body verbatim.
    assert!(
        out.contains("mod inner {\n    fn b() {}\n    use std::fmt;\n    struct S;\n}"),
        "opt-out must leave inline mod body untouched:\n{out}"
    );
}

#[test]
fn struct_before_trait_flag_propagates_into_inline_mods() {
    let input = "\
mod inner {
    trait T {}
    struct S;
}
";
    let cfg = Config {
        no_trait_before_struct: true,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    let p_struct = out.find("struct S").unwrap();
    let p_trait = out.find("trait T").unwrap();
    assert!(
        p_struct < p_trait,
        "struct_before_trait must propagate into inline mod recursion:\n{out}"
    );
}
/// `#[path = "..."]` on the inline mod doesn't change body-internal
/// reordering — the macro-as-barrier rule applies inside the body
/// regardless of where the (notional) child file would live.
#[test]
fn precise_macro_constraint_follows_inline_mod_path_attribute() {
    let td = TempDir::new("inline-precise-path");
    td.write(
        "src/elsewhere/sub/bar.rs",
        "pub fn helper() {\n    m!(\"hi\");\n}\n",
    );
    let lib = td.write(
        "src/lib.rs",
        r#"#[path = "elsewhere/sub/inline.rs"]
pub mod foo {
    macro_rules! m { ($e:expr) => { let _ = $e; }; }
    pub mod bar;
}
"#,
    );
    let src = fs::read_to_string(&lib).unwrap();
    let cfg = Config::default();
    let out = reorder_source_with_path(&src, Some(&lib), &cfg).unwrap();
    let p_macro = out.find("macro_rules! m").unwrap();
    let p_mod = out.find("pub mod bar").unwrap();
    assert!(
        p_macro < p_mod,
        "with #[path = \"elsewhere/sub/inline.rs\"] on the inline mod, child files resolve under elsewhere/sub/:\n{out}"
    );
}

/// Inside an inline mod body, a `macro_rules!` definition must
/// precede the next `mod bar;` declaration: macro items act as hard
/// barriers in their containing scope, so anything below the macro
/// in source can't sort up across it (and vice versa). No child
/// file is opened — the barrier alone enforces the constraint.
#[test]
fn precise_macro_constraint_inside_inline_mod_with_bare_caller() {
    let td = TempDir::new("inline-precise-bare");
    // Child file lives at src/foo/bar.rs because the inline mod is
    // named `foo` and the parent is `src/lib.rs`.
    td.write("src/foo/bar.rs", "pub fn helper() {\n    m!(\"hi\");\n}\n");
    let lib = td.write(
        "src/lib.rs",
        r#"pub mod foo {
    macro_rules! m { ($e:expr) => { let _ = $e; }; }
    pub mod bar;
}
"#,
    );
    let src = fs::read_to_string(&lib).unwrap();
    let cfg = Config::default();
    let out = reorder_source_with_path(&src, Some(&lib), &cfg).unwrap();
    let p_macro = out.find("macro_rules! m").unwrap();
    let p_mod = out.find("pub mod bar").unwrap();
    // Default category sort would push macro_rules to its weight-92 slot
    // (after the weight-30 `mod`). Precise scan finds the bare caller in
    // src/foo/bar.rs and yanks the macro back above the mod.
    assert!(
        p_macro < p_mod,
        "macro barrier must keep macro_rules! before `mod bar;` in source order:\n{out}"
    );
}

