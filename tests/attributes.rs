//! Attribute preservation: `#[derive(...)]`, `#[cfg(...)]`,
//! `#[cfg_attr(...)]`, `#[deprecated]`, `#[doc = "..."]`, etc. all stay
//! attached to their item across the reorder.

use cargo_reorder::reorder_source;

#[test]
fn shebang_stays_at_top() {
    let input = "#!/usr/bin/env rustc\n\nfn z() {}\nuse std::fmt;\n";
    let out = reorder_source(input).unwrap();
    assert!(out.starts_with("#!/usr/bin/env rustc\n"), "{out}");
}
#[test]
fn cfg_attr_form_preserved() {
    let input = "\
fn z() {}

#[cfg_attr(feature = \"serde\", derive(serde::Serialize, serde::Deserialize))]
pub struct Wrapped(u32);
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains(
            "#[cfg_attr(feature = \"serde\", derive(serde::Serialize, serde::Deserialize))]"
        ),
        "{out}"
    );
}

#[test]
fn cfg_gated_struct_kept_intact() {
    let input = "\
fn helper() {}

#[cfg(unix)]
pub struct UnixOnly { fd: i32 }

#[cfg(windows)]
pub struct WindowsOnly { handle: u64 }
";
    let out = reorder_source(input).unwrap();
    assert!(out.contains("#[cfg(unix)]\npub struct UnixOnly"), "{out}");
    assert!(
        out.contains("#[cfg(windows)]\npub struct WindowsOnly"),
        "{out}"
    );
    syn::parse_file(&out).unwrap();
}

#[test]
fn multi_line_derive_preserved() {
    let input = "\
fn x() {}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
)]
pub struct Multi { v: u32 }
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("#[derive(\n    Debug,\n    Clone,\n    PartialEq,\n    Eq,\n    Hash,\n)]"),
        "multi-line derive lost its formatting:\n{out}"
    );
    assert!(
        out.find("pub struct Multi").unwrap() < out.find("fn x").unwrap(),
        "{out}"
    );
}

#[test]
fn deprecated_attribute_attached() {
    let input = "fn z() {}\n#[deprecated(note = \"use Bar instead\")]\npub struct Foo;\n";
    let out = reorder_source(input).unwrap();
    let p_attr = out.find("#[deprecated").unwrap();
    let p_struct = out.find("pub struct Foo").unwrap();
    assert!(p_attr < p_struct, "{out}");
    assert_eq!(
        p_attr + 1,
        p_struct - "#[deprecated(note = \"use Bar instead\")]\n".len() + 1,
        "no separator"
    );
}

#[test]
fn inner_doc_string_attribute_form() {
    // `#[doc = "..."]` as written attribute.
    let input = "\
fn z() {}

#[doc = \"This is the foo struct.\"]
pub struct Foo;
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.contains("#[doc = \"This is the foo struct.\"]"),
        "{out}"
    );
    assert!(
        out.find("pub struct Foo").unwrap() < out.find("fn z").unwrap(),
        "{out}"
    );
}

#[test]
fn multiple_attrs_stack_above_item() {
    let input = "\
fn z() {}

#[derive(Debug)]
#[cfg(feature = \"x\")]
#[allow(dead_code)]
pub struct Stacked;
";
    let out = reorder_source(input).unwrap();
    let p_derive = out.find("#[derive(Debug)]").unwrap();
    let p_cfg = out.find("#[cfg(feature = \"x\")]").unwrap();
    let p_allow = out.find("#[allow(dead_code)]").unwrap();
    let p_struct = out.find("pub struct Stacked").unwrap();
    assert!(
        p_derive < p_cfg && p_cfg < p_allow && p_allow < p_struct,
        "stacked attrs preserved in source order:\n{out}"
    );
}

#[test]
fn crate_level_inner_attrs_stay_at_top() {
    let input = "\
#![allow(dead_code)]
#![deny(warnings)]
//! crate-level docs

fn z() {}
use std::fmt;
";
    let out = reorder_source(input).unwrap();
    assert!(
        out.starts_with("#![allow(dead_code)]\n#![deny(warnings)]\n//! crate-level docs"),
        "{out}"
    );
    assert!(
        out.find("use std::fmt").unwrap() < out.find("fn z()").unwrap(),
        "{out}"
    );
}

#[test]
fn derive_attribute_travels_with_struct() {
    let input = "fn x() {}\n#[derive(Debug, Clone, PartialEq)]\npub struct S { f: u32 }\n";
    let out = reorder_source(input).unwrap();
    let p_attr = out.find("#[derive(Debug, Clone, PartialEq)]").unwrap();
    let p_struct = out.find("pub struct S").unwrap();
    let p_fn = out.find("fn x").unwrap();
    assert!(p_attr < p_struct, "{out}");
    let between = &out[p_attr..p_struct];
    assert!(
        !between.contains("fn"),
        "fn should not be between derive and struct: {between:?}"
    );
    assert!(p_struct < p_fn, "{out}");
}

