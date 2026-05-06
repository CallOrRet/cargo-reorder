use cargo_reorder::reorder_source;
use pretty_assertions::assert_eq;

#[test]
fn doc_comment_stays_with_item() {
    let input = "\
fn before() {}

/// docs for foo
fn foo() {}

use std::fmt;
";
    let out = reorder_source(input).unwrap();
    // `use` lands first (cross-category move); the doc must travel
    // with `fn foo` wherever that ends up under within-category
    // grouping.
    let p_use = out.find("use std::fmt").unwrap();
    let p_docs = out.find("/// docs for foo").unwrap();
    let p_foo = out.find("fn foo()").unwrap();
    assert!(p_use < p_docs);
    assert!(p_use < p_foo);
    // Docs must immediately precede `fn foo()` (no other item between them).
    let between = &out[p_docs..p_foo];
    assert_eq!(between.lines().count(), 1, "docs not adjacent: {between:?}");
}

#[test]
fn line_comment_above_item_is_kept() {
    let input = "\
fn z() {}

// explanatory comment
const X: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    // const X with its comment must come before fn z, comment still attached.
    let p_comment = out.find("// explanatory comment").unwrap();
    let p_const = out.find("const X").unwrap();
    let p_fn = out.find("fn z()").unwrap();
    assert!(p_comment < p_const);
    assert!(p_const < p_fn);
    let between = &out[p_comment..p_const];
    assert_eq!(between.trim_end(), "// explanatory comment");
}

#[test]
fn block_comment_above_item_is_kept() {
    let input = "\
fn z() {}

/* block comment */
struct S;
";
    let out = reorder_source(input).unwrap();
    let p_block = out.find("/* block comment */").unwrap();
    let p_struct = out.find("struct S").unwrap();
    let p_fn = out.find("fn z()").unwrap();
    assert!(p_block < p_struct);
    assert!(p_struct < p_fn);
}

#[test]
fn inner_attrs_stay_at_top() {
    let input = "\
#![allow(dead_code)]
#![deny(warnings)]
//! crate-level docs

fn z() {}
use std::fmt;
";
    let out = reorder_source(input).unwrap();
    // Inner attrs must be the very first lines.
    assert!(
        out.starts_with("#![allow(dead_code)]\n#![deny(warnings)]\n//! crate-level docs"),
        "got:\n{out}"
    );
    assert!(out.find("use std::fmt").unwrap() < out.find("fn z()").unwrap());
}

#[test]
fn shebang_preserved() {
    let input = "#!/usr/bin/env -S cargo +nightly -Zscript\n\nfn z() {}\nuse std::fmt;\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.starts_with("#!/usr/bin/env -S cargo +nightly -Zscript\n"),
        "got:\n{out}"
    );
}

#[test]
fn comment_block_above_first_item_preserved() {
    let input = "\
// header note about this file
// it says hello

fn z() {}
use std::fmt;
";
    let out = reorder_source(input).unwrap();
    // The header comment should remain attached to the FIRST item in output
    // (which is now the `use`).
    let p_header = out.find("// header note").unwrap();
    let p_use = out.find("use std::fmt").unwrap();
    assert!(p_header < p_use);
}
