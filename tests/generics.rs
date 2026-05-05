//! Generic syntax: lifetimes, type / lifetime / const generics, where
//! clauses, GAT (generic associated types), HRTB, dyn Trait. The
//! reorderer only cares about top-level item *kinds* — these tests
//! ensure the syntax doesn't trip up our parser / span handling.

use cargo_reorder::reorder_source;

fn assert_compiles(out: &str) {
    syn::parse_file(out).expect("output must still be parseable Rust");
}

#[test]
fn generic_struct_with_lifetimes() {
    let input = "\
fn x() {}
pub struct Slice<'a, T: Clone> { data: &'a [T] }
";
    let out = reorder_source(input).unwrap();
    assert_compiles(&out);
    assert!(
        out.find("Slice<'a, T: Clone>").unwrap() < out.find("fn x").unwrap(),
        "{out}"
    );
}

#[test]
fn impl_with_where_clause() {
    let input = "\
struct Wrap<T>(T);
impl<T> Wrap<T>
where
    T: Clone + Send + 'static,
{
    fn dup(&self) -> Self
    where
        T: Default,
    {
        Wrap(T::default())
    }
}
fn z() {}
";
    let out = reorder_source(input).unwrap();
    assert_compiles(&out);
    let p_struct = out.find("struct Wrap").unwrap();
    let p_impl = out.find("impl<T> Wrap").unwrap();
    let p_fn = out.find("fn z").unwrap();
    assert!(p_struct < p_impl && p_impl < p_fn, "{out}");
}

#[test]
fn const_generics() {
    let input = "\
fn z() {}
pub struct Buffer<const N: usize> { data: [u8; N] }
impl<const N: usize> Buffer<N> {
    pub const fn new() -> Self { Buffer { data: [0; N] } }
}
";
    let out = reorder_source(input).unwrap();
    assert_compiles(&out);
    assert!(
        out.find("Buffer<const N").unwrap() < out.find("impl<const N").unwrap(),
        "{out}"
    );
    assert!(
        out.find("impl<const N").unwrap() < out.find("fn z").unwrap(),
        "{out}"
    );
}

#[test]
fn gat_in_trait() {
    let input = "\
fn x() {}
pub trait Container {
    type Item<'a> where Self: 'a;
    fn iter<'a>(&'a self) -> Self::Item<'a>;
}
";
    let out = reorder_source(input).unwrap();
    assert_compiles(&out);
    assert!(
        out.find("trait Container").unwrap() < out.find("fn x").unwrap(),
        "{out}"
    );
}

#[test]
fn hrtb_lifetime_in_bound() {
    let input = "\
fn z() {}
pub trait Callback: for<'a> Fn(&'a str) -> bool {}
";
    let out = reorder_source(input).unwrap();
    assert_compiles(&out);
    assert!(
        out.contains("for<'a> Fn(&'a str)"),
        "HRTB syntax mangled:\n{out}"
    );
}

#[test]
fn dyn_trait_in_field() {
    let input = "\
fn z() {}
pub struct Erased {
    handler: Box<dyn Fn(u32) -> bool + Send>,
}
";
    let out = reorder_source(input).unwrap();
    assert_compiles(&out);
    assert!(out.contains("Box<dyn Fn(u32) -> bool + Send>"), "{out}");
}

#[test]
fn associated_consts_and_types_in_trait() {
    let input = "\
fn z() {}
pub trait Spec {
    const MAX: usize;
    type Buffer: AsRef<[u8]>;
    fn build() -> Self::Buffer;
}
";
    let out = reorder_source(input).unwrap();
    assert_compiles(&out);
}

#[test]
fn impl_with_multiple_lifetime_and_type_params() {
    let input = "\
struct Triple<'a, 'b, T, U: 'b>(&'a T, &'b U);
impl<'a, 'b: 'a, T, U: 'b + Clone> Triple<'a, 'b, T, U> {
    fn unpack(&self) -> (&'a T, &'b U) { (self.0, self.1) }
}
fn z() {}
";
    let out = reorder_source(input).unwrap();
    assert_compiles(&out);
    let p_struct = out.find("struct Triple").unwrap();
    let p_impl = out.find("impl<'a, 'b: 'a").unwrap();
    let p_fn = out.find("fn z").unwrap();
    assert!(p_struct < p_impl && p_impl < p_fn, "{out}");
}

#[test]
fn async_trait_method_signature_preserved() {
    let input = "\
fn z() {}
pub trait AsyncRead {
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
}
";
    let out = reorder_source(input).unwrap();
    assert_compiles(&out);
    assert!(
        out.contains("async fn read"),
        "async fn in trait should remain intact:\n{out}"
    );
}
