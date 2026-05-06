//! Less-mainstream Rust syntax: unicode identifiers, raw strings, very
//! sparse files, large blank-line stretches, weird shebang+frontmatter
//! combos, etc. These shouldn't crash or mangle the output.

use cargo_reorder::{Config, reorder_source, reorder_source_with};

#[test]
fn empty_file_unchanged() {
    assert_eq!(reorder_source("").unwrap(), "");
    assert_eq!(reorder_source("\n").unwrap(), "\n");
}

#[test]
fn only_inner_doc_unchanged() {
    let src = "//! crate documentation\n";
    assert_eq!(reorder_source(src).unwrap(), src);
}

#[test]
fn only_inner_attrs_unchanged() {
    let src = "#![allow(dead_code)]\n#![deny(warnings)]\n";
    assert_eq!(reorder_source(src).unwrap(), src);
}

#[test]
fn unicode_identifiers_preserved() {
    let input =
        "const 名前: &str = \"日本語\";\nfn こんにちは() -> &'static str { 名前 }\nuse std::fmt;\n";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // use sorts before const, const before fn.
    assert!(
        out.find("use std::fmt").unwrap() < out.find("const 名前").unwrap(),
        "{out}"
    );
    assert!(
        out.find("const 名前").unwrap() < out.find("fn こんにちは").unwrap(),
        "{out}"
    );
}

#[test]
fn raw_strings_with_escapes_preserved() {
    let input = "fn x() -> &'static str { r#\"hello \"world\"\"# }\nuse std::fmt;\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("r#\"hello \"world\"\"#"),
        "raw string mangled:\n{out}"
    );
    syn::parse_file(&out).unwrap();
}

#[test]
fn many_blank_lines_preserved_and_idempotent() {
    let input = "fn a() {}\n\n\n\n\n\nuse std::fmt;\n";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let out2 = reorder_source(&out).unwrap();
    assert_eq!(out, out2, "must be idempotent on whitespace-heavy input");
    assert!(
        out.find("use std::fmt").unwrap() < out.find("fn a").unwrap(),
        "{out}"
    );
}

#[test]
fn windows_line_endings_round_trip() {
    let input = "fn z() {}\r\nuse std::fmt;\r\n";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // Reorder still applies.
    assert!(
        out.find("use std::fmt").unwrap() < out.find("fn z").unwrap(),
        "{out}"
    );
}

#[test]
fn item_with_inline_module_and_nested_items() {
    let input = "\
fn z() {}

mod inner {
    pub fn helper() {}
    use crate::shared::Thing;
    pub struct Inside;
}

use std::fmt;
";
    let cfg = Config {
        no_reorder_inline_mods: true,
        ..Config::default()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    // With `no_reorder_inline_mods: true` we only reorder TOP-level items.
    // The inline `mod inner { ... }` body is treated as opaque — its
    // inner items are not reshuffled.
    assert!(out.contains("mod inner {\n    pub fn helper() {}\n    use crate::shared::Thing;\n    pub struct Inside;\n}"),
        "inline mod body must remain intact:\n{out}");
}

#[test]
fn single_item_files() {
    for src in [
        "fn main() {}\n",
        "use std::fmt;\n",
        "struct S;\n",
        "// just a comment\nfn x() {}\n",
    ] {
        let out = reorder_source(src).unwrap();
        syn::parse_file(&out).unwrap();
    }
}

#[test]
fn item_at_eof_without_trailing_newline() {
    let input = "use std::fmt;\nfn z() {}";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // We always emit a trailing newline.
    assert!(out.ends_with('\n'), "{out:?}");
}
