//! RFC 3502 cargo-script files: an optional shebang plus a `---` ... `---`
//! TOML frontmatter block followed by ordinary Rust source. The reorderer
//! must leave the prefix verbatim and reorder only the Rust body.

use cargo_reorder::reorder_source;

#[test]
fn frontmatter_only_with_already_sorted_body() {
    let src = "\
---
[package]
edition = \"2024\"
[dependencies]
serde = \"1\"
---

use std::fmt;

fn main() {}
";
    let out = reorder_source(src).unwrap();
    assert_eq!(out, src, "must be a no-op:\n{out}");
}

#[test]
fn body_is_reordered_frontmatter_is_preserved() {
    let src = "\
---
[package]
edition = \"2024\"
---

fn helper() {}
use std::fmt;
const X: u32 = 1;
";
    let out = reorder_source(src).unwrap();
    // Frontmatter intact at the top.
    assert!(
        out.starts_with("---\n[package]\nedition = \"2024\"\n---\n"),
        "{out}"
    );
    // Body reordered: use, const, fn.
    let p_use = out.find("use std::fmt").unwrap();
    let p_const = out.find("const X").unwrap();
    let p_fn = out.find("fn helper").unwrap();
    assert!(
        p_use < p_const && p_const < p_fn,
        "body not reordered:\n{out}"
    );
}

#[test]
fn shebang_then_frontmatter() {
    let src = "\
#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = \"2024\"
---

fn z() {}
use std::fmt;
";
    let out = reorder_source(src).unwrap();
    assert!(
        out.starts_with("#!/usr/bin/env -S cargo +nightly -Zscript\n---\n"),
        "{out}"
    );
    assert!(
        out.find("use std::fmt").unwrap() < out.find("fn z").unwrap(),
        "{out}"
    );
}

#[test]
fn frontmatter_with_info_string() {
    let src = "\
---cargo
[package]
edition = \"2024\"
---

fn x() {}
use std::fmt;
";
    let out = reorder_source(src).unwrap();
    assert!(
        out.starts_with("---cargo\n[package]\nedition = \"2024\"\n---\n"),
        "{out}"
    );
    assert!(
        out.find("use std::fmt").unwrap() < out.find("fn x").unwrap(),
        "{out}"
    );
}

#[test]
fn unmatched_fence_is_left_for_syn_to_complain() {
    // Opener with 4 dashes, closer with 3 — not a valid frontmatter, the
    // splitter should bail out and `syn` will emit a parse error (which
    // the CLI then reports as a warning and skips).
    let src = "\
----cargo
[package]
---

fn main() {}
";
    let result = reorder_source(src);
    assert!(
        result.is_err(),
        "expected parse error for malformed frontmatter, got: {:?}",
        result
    );
}

#[test]
fn no_frontmatter_means_normal_handling() {
    let src = "fn main() {}\nuse std::fmt;\n";
    let out = reorder_source(src).unwrap();
    assert!(out.starts_with("use std::fmt"), "{out}");
}
