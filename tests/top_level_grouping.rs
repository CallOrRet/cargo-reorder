//! Top-level same-category grouping: structs/unions/enums/traits/fns/
//! async-fns within their own category bucket are reordered using the
//! same prefix-grouping + length-sort rules as field-level. Default
//! ON; opt out with `Config { no_reorder_fields: true, ..default }`
//! (the same flag covers both field-level and top-level grouping).

use cargo_reorder::{Config, reorder_source, reorder_source_with};

fn opt_out() -> Config {
    Config {
        no_reorder_fields: true,
        ..Config::default()
    }
}

#[test]
fn opt_out_preserves_top_level_source_order() {
    let input = "\
fn user_login() {}
fn cache_get() {}
fn user_logout() {}
";
    let out = reorder_source_with(input, &opt_out()).unwrap();
    let p_login = out.find("fn user_login").unwrap();
    let p_get = out.find("fn cache_get").unwrap();
    let p_logout = out.find("fn user_logout").unwrap();
    assert!(p_login < p_get && p_get < p_logout, "{out}");
}

#[test]
fn idempotent_after_one_pass() {
    let input = "\
fn user_logout() {}
fn cache_set() {}
fn user_login() {}
fn cache_get() {}
struct user;
struct cache;
";
    let p1 = reorder_source(input).unwrap();
    let p2 = reorder_source(&p1).unwrap();
    assert_eq!(p1, p2, "second pass must be a no-op:\n{p1}\n---\n{p2}");
}
#[test]
fn structs_drag_their_impls_along() {
    // After regrouping struct names by prefix/length, each struct's
    // impl blocks must follow it (anchor-based grouping is preserved).
    let input = "\
struct cache_layer;
struct user;
impl user { fn get(&self) {} }
impl cache_layer { fn evict(&self) {} }
";
    let out = reorder_source(input).unwrap();
    let p_user = out.find("struct user;").unwrap();
    let p_imp_user = out.find("impl user").unwrap();
    let p_cache = out.find("struct cache_layer;").unwrap();
    let p_imp_cache = out.find("impl cache_layer").unwrap();
    // user (mean 4) before cache_layer (mean 11).
    // Each struct followed by its impl.
    assert!(
        p_user < p_imp_user && p_imp_user < p_cache && p_cache < p_imp_cache,
        "{out}"
    );
}

#[test]
fn cross_category_order_unchanged() {
    // Top-level grouping only operates within a category. The
    // category boundary (struct < fn etc.) still trumps.
    let input = "\
fn process_a() {}
struct user;
struct cache;
fn process_b() {}
";
    let out = reorder_source(input).unwrap();
    // Structs (51) before fns (90), regardless of grouping.
    let p_user = out.find("struct user;").unwrap();
    let p_cache = out.find("struct cache;").unwrap();
    let p_pa = out.find("fn process_a").unwrap();
    let p_pb = out.find("fn process_b").unwrap();
    // user (4) before cache (5).
    assert!(p_user < p_cache, "{out}");
    // both structs before both fns.
    assert!(p_cache < p_pa && p_cache < p_pb, "{out}");
    // process_a, process_b same group + same length → source order.
    assert!(p_pa < p_pb, "{out}");
}

#[test]
fn fn_and_async_fn_are_separate_buckets() {
    // Fn (90) and AsyncFn (91) are different categories; grouping
    // happens within each independently.
    let input = "\
async fn user_fetch() {}
fn cache_get() {}
fn user_get() {}
async fn cache_warm() {}
";
    let out = reorder_source(input).unwrap();
    // All sync fns first (cat 90), then async fns (cat 91).
    let p_cget = out.find("fn cache_get").unwrap();
    let p_uget = out.find("fn user_get").unwrap();
    let p_uf = out.find("async fn user_fetch").unwrap();
    let p_cw = out.find("async fn cache_warm").unwrap();
    // sync fns: cache (mean 9) before user (mean 8)? Let me recompute:
    //   cache_get (9), user_get (8). Different prefixes. cache mean=9, user mean=8.
    //   user before cache.
    assert!(
        p_uget < p_cget,
        "user (mean 8) before cache (mean 9):\n{out}"
    );
    // async fns: user_fetch (10), cache_warm (10). Different prefixes,
    //   both mean 10 → source order: user_fetch (was first) before cache_warm.
    assert!(p_uf < p_cw, "{out}");
    // Both syncs before both asyncs.
    assert!(p_cget < p_uf && p_uget < p_cw, "{out}");
}

#[test]
fn top_level_fns_regroup_by_prefix() {
    let input = "\
fn user_login() {}
fn cache_get() {}
fn user_logout() {}
fn cache_set() {}
";
    let out = reorder_source(input).unwrap();
    // Group `cache`: cache_get (9), cache_set (9) → mean 9
    // Group `user`: user_login (10), user_logout (11) → mean 10.5
    // cache group first.
    let p_cget = out.find("fn cache_get").unwrap();
    let p_cset = out.find("fn cache_set").unwrap();
    let p_ulogin = out.find("fn user_login").unwrap();
    let p_ulogout = out.find("fn user_logout").unwrap();
    assert!(
        p_cget < p_cset && p_cset < p_ulogin && p_ulogin < p_ulogout,
        "{out}"
    );
}

#[test]
fn top_level_structs_regroup_by_prefix() {
    let input = "\
struct cache_dir;
struct user;
struct cache_size;
struct user_id;
";
    let out = reorder_source(input).unwrap();
    // Group `user`: user (4), user_id (7) → mean 5.5
    // Group `cache`: cache_dir (9), cache_size (10) → mean 9.5
    // user group first (smaller mean).
    let p_user = out.find("struct user;").unwrap();
    let p_uid = out.find("struct user_id").unwrap();
    let p_cdir = out.find("struct cache_dir").unwrap();
    let p_csize = out.find("struct cache_size").unwrap();
    assert!(
        p_user < p_uid && p_uid < p_cdir && p_cdir < p_csize,
        "{out}"
    );
}

#[test]
fn top_level_async_fns_regroup_by_prefix() {
    let input = "\
async fn user_fetch() {}
async fn cache_warm() {}
async fn user_save() {}
async fn cache_purge() {}
";
    let out = reorder_source(input).unwrap();
    // user group: user_save (9), user_fetch (10) → mean 9.5
    // cache group: cache_warm (10), cache_purge (11) → mean 10.5
    // user group first; within each group, shorter first.
    let p_us = out.find("async fn user_save").unwrap();
    let p_uf = out.find("async fn user_fetch").unwrap();
    let p_cw = out.find("async fn cache_warm").unwrap();
    let p_cp = out.find("async fn cache_purge").unwrap();
    assert!(p_us < p_uf && p_uf < p_cw && p_cw < p_cp, "{out}");
}

#[test]
fn top_level_enums_regroup_by_pascal_prefix() {
    let input = "\
enum BarApple { A }
enum FooLong { B }
enum Foo { C }
enum BarBanana { D }
";
    let out = reorder_source(input).unwrap();
    // Foo group: Foo (3), FooLong (7) → mean 5
    // Bar group: BarApple (8), BarBanana (9) → mean 8.5
    // Foo first.
    let p_foo = out.find("enum Foo {").unwrap();
    let p_foolong = out.find("enum FooLong").unwrap();
    let p_apple = out.find("enum BarApple").unwrap();
    let p_banana = out.find("enum BarBanana").unwrap();
    assert!(
        p_foo < p_foolong && p_foolong < p_apple && p_apple < p_banana,
        "{out}"
    );
}

#[test]
fn top_level_traits_regroup_by_pascal_prefix() {
    let input = "\
trait CacheLayer {}
trait RenderEngine {}
trait CacheStore {}
trait RenderQueue {}
";
    let out = reorder_source(input).unwrap();
    // Cache group: CacheStore (10), CacheLayer (10) → mean 10
    // Render group: RenderQueue (11), RenderEngine (12) → mean 11.5
    // Cache group first; within group ties preserve source order.
    let p_clayer = out.find("trait CacheLayer").unwrap();
    let p_cstore = out.find("trait CacheStore").unwrap();
    let p_rqueue = out.find("trait RenderQueue").unwrap();
    let p_rengine = out.find("trait RenderEngine").unwrap();
    // CacheLayer was first in source, so among ties (both 10) it goes first.
    assert!(p_clayer < p_cstore, "{out}");
    // Cache group entirely before Render group.
    assert!(p_cstore < p_rqueue && p_cstore < p_rengine, "{out}");
    // Within Render: RenderEngine (12) > RenderQueue (11) → queue first.
    assert!(p_rqueue < p_rengine, "{out}");
}

