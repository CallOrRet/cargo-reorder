//! `impl` and `trait` body recursion: members get the same prefix-
//! group + length sort as struct fields and top-level same-category
//! items, plus a category sub-bucket so `const < type < fn < async fn`
//! (mirroring the top-level category order). Macro / verbatim members
//! are hard barriers — their presence leaves the whole body
//! verbatim.

use cargo_reorder::{Config, reorder_source, reorder_source_with};

fn opt_out() -> Config {
    Config {
        no_reorder_fields: true,
        ..Config::default()
    }
}

#[test]
fn opt_out_preserves_impl_body_source_order() {
    let input = "\
impl Foo {
    fn user_logout(&self) {}
    fn cache_get(&self) {}
    fn user_login(&self) {}
}
";
    let out = reorder_source_with(input, &opt_out()).unwrap();
    let p_logout = out.find("fn user_logout").unwrap();
    let p_get = out.find("fn cache_get").unwrap();
    let p_login = out.find("fn user_login").unwrap();
    assert!(p_logout < p_get && p_get < p_login, "{out}");
}

#[test]
fn idempotent_after_one_pass() {
    let input = "\
impl Cache {
    fn user_logout(&self) {}
    fn cache_get(&self) {}
    fn user_login(&self) {}
    fn cache_set(&self) {}
}
";
    let p1 = reorder_source(input).unwrap();
    let p2 = reorder_source(&p1).unwrap();
    assert_eq!(p1, p2, "second pass must be a no-op:\n{p1}\n---\n{p2}");
}

#[test]
fn empty_impl_body_unaffected() {
    let input = "impl Foo {}\n";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

#[test]
fn single_method_impl_unaffected() {
    let input = "\
impl Foo {
    fn only(&self) {}
}
";
    let out = reorder_source(input).unwrap();
    assert_eq!(out, input);
}

fn impl_fns_off() -> Config {
    Config {
        no_reorder_impl_fns: true,
        ..Config::default()
    }
}

#[test]
fn impl_body_full_category_chain() {
    // Members in all four buckets: const → type → fn → async fn.
    let input = "\
impl Foo {
    fn b(&self) {}
    type Item = u32;
    async fn warm(&self) {}
    const KEY_A: u32 = 0;
    fn a(&self) {}
    const KEY_B: u32 = 1;
}
";
    let out = reorder_source(input).unwrap();
    let p_ka = out.find("const KEY_A").unwrap();
    let p_kb = out.find("const KEY_B").unwrap();
    let p_type = out.find("type Item").unwrap();
    let p_a = out.find("fn a").unwrap();
    let p_b = out.find("fn b").unwrap();
    let p_warm = out.find("async fn warm").unwrap();
    // const < type < fn < async-fn.
    assert!(
        p_ka < p_kb && p_kb < p_type,
        "consts grouped before type:\n{out}"
    );
    // `a` and `b` are each single-letter groups, mean 1 each;
    // tied means → source order: b first (idx 0), a fourth (idx 4).
    assert!(p_type < p_b && p_b < p_a, "type before fns:\n{out}");
    assert!(p_a < p_warm, "fn before async fn:\n{out}");
}

#[test]
fn impl_body_methods_regroup_by_prefix() {
    let input = "\
impl Foo {
    fn user_logout(&self) {}
    fn cache_get(&self) {}
    fn user_login(&self) {}
    fn cache_set(&self) {}
}
";
    let out = reorder_source(input).unwrap();
    // cache group (mean 9) before user group (mean 10.5); within each
    // group sorted by length (ties by source order).
    let p_cget = out.find("fn cache_get").unwrap();
    let p_cset = out.find("fn cache_set").unwrap();
    let p_ulogin = out.find("fn user_login").unwrap();
    let p_ulogout = out.find("fn user_logout").unwrap();
    assert!(
        p_cget < p_cset && p_cset < p_ulogin && p_ulogin < p_ulogout,
        "{out}"
    );
    // Blank line between cache and user groups.
    let between = &out[p_cset..p_ulogin];
    assert!(
        between.contains("\n\n"),
        "blank line between groups:\n{out}"
    );
}

#[test]
fn impl_body_sync_fns_before_async_fns() {
    // Inside an impl block, all sync `fn` come before all `async fn`
    // — same convention as top-level Fn (90) before AsyncFn (91).
    // Within each bucket, the prefix-group + length rules apply.
    let input = "\
impl Cache {
    async fn fetch(&self) {}
    fn store(&self) {}
    async fn refresh(&self) {}
    fn evict(&self) {}
}
";
    let out = reorder_source(input).unwrap();
    let p_store = out.find("fn store").unwrap();
    let p_evict = out.find("fn evict").unwrap();
    let p_fetch = out.find("async fn fetch").unwrap();
    let p_refresh = out.find("async fn refresh").unwrap();
    // Both sync fns precede both async fns.
    assert!(p_store < p_fetch && p_store < p_refresh, "{out}");
    assert!(p_evict < p_fetch && p_evict < p_refresh, "{out}");
    // Within sync bucket: store and evict each in its own one-element
    // group (mean 5 each). Tied → source order. store was first.
    assert!(p_store < p_evict, "tied length → source order:\n{out}");
    // Within async bucket: fetch (5) before refresh (7).
    assert!(p_fetch < p_refresh, "{out}");
}

#[test]
fn impl_body_with_attributes_keeps_them_attached() {
    let input = "\
impl Foo {
    #[inline]
    fn user_save(&self) {}
    fn cache_clear(&self) {}
    #[deprecated]
    fn user_load(&self) {}
}
";
    let out = reorder_source(input).unwrap();
    assert!(out.contains("#[inline]\n    fn user_save"), "{out}");
    assert!(out.contains("#[deprecated]\n    fn user_load"), "{out}");
}

#[test]
fn impl_body_with_const_and_type_sorts_by_category() {
    // const < type < fn < async-fn — same convention as top-level
    // category ordering. Within each bucket, prefix-group + length
    // sort applies.
    let input = "\
impl Foo {
    type Item = u32;
    const KEY: u32 = 0;
    fn user_b(&self) {}
    fn user_a(&self) {}
}
";
    let out = reorder_source(input).unwrap();
    let p_const = out.find("const KEY").unwrap();
    let p_type = out.find("type Item").unwrap();
    let p_user_b = out.find("fn user_b").unwrap();
    let p_user_a = out.find("fn user_a").unwrap();
    assert!(p_const < p_type, "const before type:\n{out}");
    assert!(
        p_type < p_user_b && p_type < p_user_a,
        "type before fns:\n{out}"
    );
    // Within fn bucket: same group "user", same length 6 — tied,
    // source order preserves user_b before user_a.
    assert!(p_user_b < p_user_a, "{out}");
}

#[test]
fn impl_body_with_doc_comments_keeps_them_attached() {
    let input = "\
impl Foo {
    /// docs for user_save
    fn user_save(&self) {}
    /// docs for cache_clear
    fn cache_clear(&self) {}
    fn user_load(&self) {}
}
";
    let out = reorder_source(input).unwrap();
    // Each `///` line must immediately precede its fn.
    assert!(
        out.contains("/// docs for user_save\n    fn user_save"),
        "{out}"
    );
    assert!(
        out.contains("/// docs for cache_clear\n    fn cache_clear"),
        "{out}"
    );
}

#[test]
fn nested_impl_inside_inline_mod_recurses() {
    // Inline-mod recursion + impl-body recursion compose: the
    // top-level mod gets walked, items inside (including impl) get
    // their bodies reordered. user_a and user_b are both length 6
    // (tied), so stable sort preserves source order: user_short
    // comes first, then the longer one moves second.
    let input = "\
mod inner {
    impl Foo {
        fn user_long_name(&self) {}
        fn user_a(&self) {}
    }
}
";
    let out = reorder_source(input).unwrap();
    let p_a = out.find("fn user_a").unwrap();
    let p_long = out.find("fn user_long_name").unwrap();
    assert!(p_a < p_long, "shorter user name comes first:\n{out}");
}
#[test]
fn trait_body_methods_regroup_by_prefix() {
    let input = "\
trait Repo {
    fn save_record(&self);
    fn delete_record(&self);
    fn save_index(&self);
}
";
    let out = reorder_source(input).unwrap();
    // save group: save_index (10), save_record (11) → mean 10.5
    // delete group: delete_record (13) → mean 13
    // save group first.
    let p_sindex = out.find("fn save_index").unwrap();
    let p_srec = out.find("fn save_record").unwrap();
    let p_drec = out.find("fn delete_record").unwrap();
    assert!(p_sindex < p_srec && p_srec < p_drec, "{out}");
}

#[test]
fn trait_body_with_associated_type_sorts_by_category() {
    let input = "\
trait Repo {
    fn save(&self, id: Self::Id);
    type Id;
    fn load(&self, id: Self::Id);
}
";
    let out = reorder_source(input).unwrap();
    let p_type = out.find("type Id").unwrap();
    let p_save = out.find("fn save").unwrap();
    let p_load = out.find("fn load").unwrap();
    // type Id (bucket 1) before fns (bucket 2).
    assert!(
        p_type < p_save && p_type < p_load,
        "type before fns:\n{out}"
    );
    // Within fn bucket: load (4-char prefix "load", mean 4)
    // before save (4-char prefix "save", mean 4) — tied means;
    // source order: save was first (idx 0), so save first.
    assert!(p_save < p_load, "tied means → source order:\n{out}");
}

#[test]
fn no_reorder_impl_fns_does_not_disable_field_reorder() {
    // The new flag is scoped to impl/trait bodies. Struct fields,
    // enum variants, and top-level same-category grouping all stay
    // under `no_reorder_fields`.
    let input = "\
struct S {
    user_loooong: u32,
    bar_x: u8,
}
";
    let out = reorder_source_with(input, &impl_fns_off()).unwrap();
    let p_bar = out.find("bar_x").unwrap();
    let p_user = out.find("user_loooong").unwrap();
    assert!(
        p_bar < p_user,
        "struct fields must still reorder under no_reorder_impl_fns:\n{out}"
    );
}

#[test]
fn no_reorder_impl_fns_keeps_impl_body_in_source_order() {
    let input = "\
impl Foo {
    fn user_login(&self) {}
    fn cache_get(&self) {}
}
";
    let out = reorder_source_with(input, &impl_fns_off()).unwrap();
    let p_login = out.find("fn user_login").unwrap();
    let p_get = out.find("fn cache_get").unwrap();
    assert!(p_login < p_get, "impl body must be source-order:\n{out}");
}

#[test]
fn no_reorder_impl_fns_keeps_trait_body_in_source_order() {
    let input = "\
trait Repo {
    fn save_record(&self);
    fn delete_record(&self);
    fn save_index(&self);
}
";
    let out = reorder_source_with(input, &impl_fns_off()).unwrap();
    let p_save = out.find("fn save_record").unwrap();
    let p_del = out.find("fn delete_record").unwrap();
    let p_idx = out.find("fn save_index").unwrap();
    assert!(p_save < p_del && p_del < p_idx, "{out}");
}

