//! `impl` block ordering & classification: anchoring (to type / to trait /
//! orphan bucket) and the four-tier inherent → std → crate → external split.

use cargo_reorder::{Config, reorder_source, reorder_source_with};

#[test]
fn std_via_use_import_recognised() {
    let input = "\
use std::fmt::Display;

struct Foo;
impl other::Trait for Foo {}
impl Display for Foo {}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("impl Display").unwrap() < out.find("impl other::Trait").unwrap(),
        "{out}"
    );
}

#[test]
fn impls_anchor_to_their_target_type() {
    let input = "\
struct A;
struct B;
impl B { fn b() {} }
impl A { fn a() {} }
";
    let out = reorder_source(input).unwrap();
    let p_a = out.find("struct A;").unwrap();
    let p_impl_a = out.find("impl A").unwrap();
    let p_b = out.find("struct B;").unwrap();
    let p_impl_b = out.find("impl B").unwrap();
    assert!(p_a < p_impl_a && p_impl_a < p_b && p_b < p_impl_b, "{out}");
}

#[test]
fn std_via_renamed_use_import_recognised() {
    let input = "\
use std::fmt::Display as Disp;

struct Foo;
impl other::Trait for Foo {}
impl Disp for Foo {}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("impl Disp").unwrap() < out.find("impl other::Trait").unwrap(),
        "{out}"
    );
}

#[test]
fn unsafe_impl_works_like_normal_impl() {
    let input = "\
struct Foo;
unsafe impl Send for Foo {}
impl Foo { fn me(&self) {} }
";
    let out = reorder_source(input).unwrap();
    let p_struct = out.find("struct Foo").unwrap();
    let p_inh = out.find("impl Foo {").unwrap();
    let p_unsafe = out.find("unsafe impl Send").unwrap();
    assert!(p_struct < p_inh && p_inh < p_unsafe, "{out}");
}

#[test]
fn unanchored_impl_falls_to_impl_bucket() {
    // Neither Display nor u32 is local — falls into Impl bucket
    // (weight 70: between Trait and ForeignMod).
    let input = "\
fn z() {}
impl std::fmt::Display for u32 {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }
}
struct Other;
";
    let out = reorder_source(input).unwrap();
    let p_struct = out.find("struct Other").unwrap();
    let p_orphan = out.find("impl std::fmt::Display").unwrap();
    let p_fn = out.find("fn z()").unwrap();
    assert!(p_struct < p_orphan && p_orphan < p_fn, "{out}");
}

#[test]
fn impl_follows_its_struct() {
    let input = "impl Foo {}\nstruct Foo;\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("struct Foo").unwrap() < out.find("impl Foo").unwrap(),
        "{out}"
    );
}

#[test]
fn impl_with_generics_anchors_correctly() {
    let input = "\
struct Wrapper<T>(T);
impl<T: Clone> Wrapper<T> {
    fn dup(&self) -> Self where T: Clone { Wrapper(self.0.clone()) }
}
fn helper() {}
";
    let out = reorder_source(input).unwrap();
    let p_struct = out.find("struct Wrapper").unwrap();
    let p_impl = out.find("impl<T: Clone> Wrapper").unwrap();
    let p_fn = out.find("fn helper").unwrap();
    assert!(p_struct < p_impl && p_impl < p_fn, "{out}");
}

#[test]
fn impl_anchors_to_local_trait_when_target_type_is_remote() {
    let input = "\
fn z() {}

trait Greet { fn greet(&self); }
impl Greet for u32 { fn greet(&self) {} }
impl Greet for String { fn greet(&self) {} }
";
    let out = reorder_source(input).unwrap();
    let p_trait = out.find("trait Greet").unwrap();
    let p_impl_u32 = out.find("impl Greet for u32").unwrap();
    let p_impl_str = out.find("impl Greet for String").unwrap();
    let p_fn = out.find("fn z()").unwrap();
    assert!(
        p_trait < p_impl_u32 && p_impl_u32 < p_impl_str,
        "trait then both impls:\n{out}"
    );
    assert!(p_impl_str < p_fn, "trait+impls block before fn:\n{out}");
}

#[test]
fn short_trait_first_can_be_disabled() {
    // With the flag off, the two impls tie on every sort key field and
    // fall through to original_index — i.e. preserve source order.
    let input = "\
struct Foo;
impl std::default::Default for Foo { fn default() -> Self { Foo } }
impl Default for Foo { fn default() -> Self { Foo } }
";
    let cfg = Config {
        no_short_trait_first: true,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    let p_long = out.find("impl std::default::Default for Foo").unwrap();
    let p_short = out.find("impl Default for Foo").unwrap();
    assert!(
        p_long < p_short,
        "with flag off, source order should be preserved:\n{out}"
    );
}
#[test]
fn short_trait_path_sorts_before_long_path() {
    // `impl Default for Foo` and `impl std::default::Default for Foo`
    // classify identically (Default is a prelude trait → StdTrait, and
    // `std::...` also classifies as StdTrait). With the default
    // `short_trait_first` on, the unqualified form sorts first.
    let input = "\
struct Foo;
impl std::default::Default for Foo { fn default() -> Self { Foo } }
impl Default for Foo { fn default() -> Self { Foo } }
";
    let out = reorder_source(input).unwrap();
    let p_short = out.find("impl Default for Foo").unwrap();
    let p_long = out.find("impl std::default::Default for Foo").unwrap();
    assert!(
        p_short < p_long,
        "shorter trait path must precede the qualified form:\n{out}"
    );
}

#[test]
fn prelude_trait_recognised_without_import() {
    // `Default`/`Clone`/`From`/`Iterator` are in the Rust prelude — auto
    // imported by the compiler, no `use` needed in source.
    let input = "\
struct Foo;
impl other::Trait for Foo {}
impl Default for Foo { fn default() -> Self { Foo } }
impl Clone for Foo { fn clone(&self) -> Self { Foo } }
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("impl Default").unwrap() < out.find("impl other::Trait").unwrap(),
        "{out}"
    );
    assert!(
        out.find("impl Clone").unwrap() < out.find("impl other::Trait").unwrap(),
        "{out}"
    );
}

#[test]
fn local_trait_classifies_as_crate_not_external() {
    let input = "\
trait MyLocal {}
struct Foo;
impl other::Trait for Foo {}
impl MyLocal for Foo {}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("impl MyLocal for Foo").unwrap() < out.find("impl other::Trait for Foo").unwrap(),
        "{out}"
    );
}

#[test]
fn non_prelude_std_trait_without_import_falls_to_external() {
    // `Display` is NOT in the prelude — without `use std::fmt::Display;`
    // we can't tell it's std, so it falls to External (same bucket as
    // `other::Trait`). With `short_trait_first` on by default the
    // single-segment `Display` sorts before the two-segment `other::Trait`;
    // both being External is what's being asserted, the short-first
    // ordering is the visible consequence.
    let input = "\
struct Foo;
impl other::Trait for Foo {}
impl Display for Foo {}
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.find("impl Display").unwrap() < out.find("other::Trait").unwrap(),
        "{out}"
    );
}

#[test]
fn inherent_then_std_via_path_then_crate_local_then_external() {
    let input = "\
struct Foo;

trait MyLocal {}

impl other_crate::Trait for Foo {}
impl MyLocal for Foo {}
impl std::fmt::Display for Foo {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }
}
impl Foo { fn me(&self) {} }
";
    let out = reorder_source(input).unwrap();
    let p_struct = out.find("struct Foo;").unwrap();
    let p_inh = out.find("impl Foo {").unwrap();
    let p_std = out.find("impl std::fmt::Display").unwrap();
    let p_crate = out.find("impl MyLocal for Foo").unwrap();
    let p_ext = out.find("impl other_crate::Trait").unwrap();
    assert!(p_struct < p_inh, "{out}");
    assert!(p_inh < p_std, "inherent < std:\n{out}");
    assert!(p_std < p_crate, "std < crate-local:\n{out}");
    assert!(p_crate < p_ext, "crate-local < external:\n{out}");
}
