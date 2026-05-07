//! Round-trip / idempotence tests: every reordered file must be valid Rust
//! and reordering it again must produce identical output.

use cargo_reorder::reorder_source;

fn assert_roundtrip(input: &str) {
    let out = reorder_source(input).expect("first reorder");
    syn::parse_file(&out).expect("output must still be parseable Rust");
    let out2 = reorder_source(&out).expect("second reorder");
    assert_eq!(
        out, out2,
        "reorder must be idempotent\n--first--\n{out}\n--second--\n{out2}"
    );
}

#[test]
fn macro_rules_roundtrip() {
    assert_roundtrip(
        r#"
fn x() {}

macro_rules! my_mac {
    () => {};
    ($e:expr) => { $e + 1 };
}

use std::fmt;
"#,
    );
}

#[test]
fn raw_strings_preserved() {
    let input = "fn x() -> &'static str { r#\"hello \"world\"\"# }\nuse std::fmt;\n";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("r#\"hello \"world\"\"#"),
        "raw string mangled:\n{out}"
    );
    syn::parse_file(&out).unwrap();
}

#[test]
fn complex_file_roundtrips() {
    assert_roundtrip(
        r#"
//! crate docs

#![allow(dead_code)]
#![deny(warnings)]

use std::collections::HashMap;
use serde::Serialize;
use crate::inner::Thing;

/// helpful constant
pub const ANSWER: u32 = 42;

static mut COUNTER: u32 = 0;

type StringMap = HashMap<String, String>;

#[derive(Debug, Clone)]
pub struct Foo {
    name: String,
}

pub enum Color {
    Red,
    Green,
    Blue,
}

pub trait Greeter {
    fn greet(&self) -> String;
}

impl Greeter for Foo {
    fn greet(&self) -> String {
        format!("hi {}", self.name)
    }
}

impl Foo {
    pub fn new(name: String) -> Self {
        Foo { name }
    }
}

// utility
fn helper() -> i32 { 1 }

async fn fetch() -> Result<(), ()> { Ok(()) }

pub mod nested;

extern crate alloc;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn it_works() {
        assert_eq!(ANSWER, 42);
    }
}
"#,
    );
}

#[test]
fn empty_module_roundtrips() {
    assert_roundtrip("");
    assert_roundtrip("\n");
    assert_roundtrip("//! just docs\n");
}

#[test]
fn multi_line_use_roundtrips() {
    assert_roundtrip(
        r#"
use std::{
    collections::{HashMap, HashSet},
    fmt::{self, Debug, Display},
};
use serde::{
    de::DeserializeOwned,
    Serialize,
};

fn x() {}
"#,
    );
}

#[test]
fn cfg_gated_items_roundtrip() {
    assert_roundtrip(
        r#"
fn main() {}

#[cfg(unix)]
mod platform {
    pub fn name() -> &'static str { "unix" }
}

#[cfg(windows)]
mod platform {
    pub fn name() -> &'static str { "windows" }
}

use std::fmt;
"#,
    );
}

#[test]
fn derive_attrs_kept_with_struct() {
    let input = "fn x() {}\n#[derive(Debug, Clone, PartialEq)]\npub struct S {\n    f: u32,\n}\n";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // The derive attr must appear directly above `pub struct S` in the output.
    let p_attr = out.find("#[derive(Debug, Clone, PartialEq)]").unwrap();
    let p_struct = out.find("pub struct S").unwrap();
    assert!(p_attr < p_struct);
    let between = &out[p_attr..p_struct];
    assert!(
        !between.contains("fn"),
        "fn should not be between derive and struct: {between:?}"
    );
}

#[test]
fn unicode_identifiers_and_strings() {
    assert_roundtrip(
        r#"
const 名前: &str = "日本語";
fn こんにちは() -> &'static str { 名前 }
use std::fmt;
"#,
    );
}

#[test]
fn many_blank_lines_preserved_and_parses() {
    let input = "fn a() {}\n\n\n\nuse std::fmt;\n";
    let out = reorder_source(input).unwrap();
    // Reorder must produce valid Rust and be idempotent regardless of how
    // many blank lines the user had.
    syn::parse_file(&out).unwrap();
    let out2 = reorder_source(&out).unwrap();
    assert_eq!(out, out2);
    assert!(out.find("use std::fmt").unwrap() < out.find("fn a()").unwrap());
}
