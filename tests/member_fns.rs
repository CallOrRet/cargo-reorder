use cargo_reorder::reorder_source;

#[test]
fn impl_fn_and_async_fn_are_sorted() {
    let input = r#"
impl Api {
    async fn user_fetch(&self) {}
    fn cache_purge(&self) {}
    fn user_save(&self) {}
    async fn cache_warm(&self) {}
}
"#;
    let out = reorder_source(input).unwrap();
    let p_cp = out.find("fn cache_purge").unwrap();
    let p_us = out.find("fn user_save").unwrap();
    let p_cw = out.find("async fn cache_warm").unwrap();
    let p_uf = out.find("async fn user_fetch").unwrap();
    assert!(p_us < p_cp && p_cp < p_uf && p_uf < p_cw, "{out}");
}

#[test]
fn trait_methods_sorted_without_crossing_consts() {
    let input = r#"
trait Service {
    async fn user_fetch(&self);
    const ID: u8;
    fn cache_purge(&self);
    fn user_save(&self);
}
"#;
    let out = reorder_source(input).unwrap();
    let p_const = out.find("const ID").unwrap();
    let p_cp = out.find("fn cache_purge").unwrap();
    let p_us = out.find("fn user_save").unwrap();
    assert!(p_const < p_us && p_us < p_cp, "{out}");
}
