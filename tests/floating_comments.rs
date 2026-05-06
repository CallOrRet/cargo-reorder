//! Coverage for "floating-comment fences": a `//`-line comment block
//! sandwiched between blank lines on both sides is treated as a section
//! divider. The comment text stays anchored to its source position and
//! items above the fence cannot reorder past items below it (and vice
//! versa). Doc comments (`///`, `//!`) are excluded — those normally
//! attach to a sibling item via syn's attribute parsing.

use cargo_reorder::{Config, reorder_source, reorder_source_with};

fn cfg() -> Config {
    Config::default()
}

/// Helper: assert `out` is byte-for-byte stable under a second pass.
fn assert_idempotent(out: &str) {
    let again = reorder_source(out).unwrap();
    assert_eq!(
        out, again,
        "not idempotent:\nfirst:\n{out}\nsecond:\n{again}"
    );
}

#[test]
fn fence_keeps_items_above_above() {
    // Without a fence, `mod support;` would move below `use std::fmt;`
    // (default ordering: extern crate -> use -> mod). The TODO comment
    // sandwiched by blank lines acts as a fence, pinning `mod support;`
    // to the segment above.
    let input = "\
extern crate test;
mod support;

// TODO: rewrite this section

use std::fmt;
fn z() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let mod_pos = out.find("mod support;").unwrap();
    let todo_pos = out.find("// TODO").unwrap();
    let use_pos = out.find("use std::fmt").unwrap();
    assert!(
        mod_pos < todo_pos && todo_pos < use_pos,
        "mod must stay above the fence and use must stay below:\n{out}"
    );
    assert_idempotent(&out);
}

#[test]
fn fence_keeps_items_below_below() {
    // A struct defined after the fence must stay after it even though
    // alphabetic / category sort would put `use std::fmt` before it.
    let input = "\
fn first_above() {}

// === section break ===

use std::fmt;
struct S;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let above = out.find("first_above").unwrap();
    let fence = out.find("section break").unwrap();
    let use_pos = out.find("use std::fmt").unwrap();
    let struct_pos = out.find("struct S").unwrap();
    assert!(above < fence, "fn must stay above fence:\n{out}");
    assert!(
        fence < use_pos && use_pos < struct_pos,
        "use and struct must both stay below fence in normal order:\n{out}"
    );
    assert_idempotent(&out);
}

#[test]
fn multi_line_comment_block_acts_as_one_fence() {
    let input = "\
fn a() {}

// section header line 1
// section header line 2
// section header line 3

use std::fmt;
fn b() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // All three comment lines stay together, sandwiched by one blank
    // each side, between the two halves.
    assert!(
        out.contains(
            "// section header line 1\n// section header line 2\n// section header line 3"
        ),
        "comment block must remain contiguous:\n{out}"
    );
    let a = out.find("fn a()").unwrap();
    let h = out.find("section header line 1").unwrap();
    let u = out.find("use std::fmt").unwrap();
    assert!(a < h && h < u, "ordering wrong:\n{out}");
    assert_idempotent(&out);
}

#[test]
fn multiple_fences_partition_into_three_sections() {
    let input = "\
fn upper_a() {}
fn upper_b() {}

// --- section 1 ---

fn middle() {}

// --- section 2 ---

use std::fmt;
fn lower() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let ua = out.find("upper_a").unwrap();
    let ub = out.find("upper_b").unwrap();
    let f1 = out.find("section 1").unwrap();
    let mid = out.find("middle").unwrap();
    let f2 = out.find("section 2").unwrap();
    let u = out.find("use std::fmt").unwrap();
    let lo = out.find("lower").unwrap();
    assert!(
        ua < ub,
        "siblings within section keep relative order:\n{out}"
    );
    assert!(ub < f1, "section 1 comes after upper section:\n{out}");
    assert!(f1 < mid && mid < f2, "middle is between fences:\n{out}");
    assert!(f2 < u && u < lo, "section 2 contents below it:\n{out}");
    assert_idempotent(&out);
}

#[test]
fn doc_comment_is_not_a_fence() {
    // `///` is an outer doc comment; syn associates it with the next
    // item as an attribute, so it should never even reach our fence
    // detector. The next item must still be allowed to move (so this
    // comment effectively migrates with `struct B`, which is the
    // correct semantics for a doc comment).
    let input = "\
struct B;

/// docs for next
struct A;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // No fence semantics: alphabetical sort within Struct bucket would
    // put A first if any tie-breaking applies. We don't actually sort
    // structs alphabetically here, but the key check is the doc stays
    // glued to its struct.
    let docs_pos = out.find("/// docs for next").unwrap();
    let a_pos = out.find("struct A").unwrap();
    let between = &out[docs_pos..a_pos];
    assert!(
        !between.contains("struct"),
        "doc must stay immediately above its struct:\n{out}"
    );
}

#[test]
fn comment_without_surrounding_blanks_is_not_a_fence() {
    // Comment immediately above an item (no blank between) is the
    // item's leading trivia — not a floating fence.
    let input = "\
fn z() {}
// inline comment for y
fn y() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // Comment moves with its anchor item; reorder is free.
    let comment_pos = out.find("// inline comment").unwrap();
    let y_pos = out.find("fn y").unwrap();
    let between = &out[comment_pos..y_pos];
    assert!(
        between.lines().count() <= 2,
        "comment must stay glued to its item:\n{out}"
    );
}

#[test]
fn fence_with_explicit_recurse_off_still_works_at_top_level() {
    // Top-level fences should work regardless of inline-mod recursion.
    let input = "\
fn upper() {}

// fence

use std::fmt;
";
    let cfg = Config {
        no_reorder_inline_mods: true,
        ..cfg()
    };
    let out = reorder_source_with(input, &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let upper = out.find("fn upper").unwrap();
    let fence = out.find("// fence").unwrap();
    let use_pos = out.find("use std::fmt").unwrap();
    assert!(
        upper < fence && fence < use_pos,
        "top-level fence broken:\n{out}"
    );
}

#[test]
fn fence_inside_inline_mod_when_recursed() {
    // With inline-mod recursion on (the default), fences inside an
    // inline mod body must also be honored.
    let input = "\
mod inner {
    fn a() {}

    // fence inside

    use std::fmt;
    fn b() {}
}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let a_pos = out.find("fn a").unwrap();
    let fence_pos = out.find("// fence inside").unwrap();
    let use_pos = out.find("use std::fmt").unwrap();
    assert!(
        a_pos < fence_pos && fence_pos < use_pos,
        "inline-mod fence semantics not preserved:\n{out}"
    );
    assert_idempotent(&out);
}

#[test]
fn fence_between_two_use_blocks_is_preserved() {
    // Splitting `use` items into two visual groups via a comment
    // divider is a real pattern (e.g. "// std" / "// third-party").
    let input = "\
use std::fmt;
use std::io;

// third-party imports below

use anyhow::Result;
use serde::Serialize;

fn x() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let std_io = out.find("use std::io").unwrap();
    let fence = out.find("third-party").unwrap();
    let anyhow_pos = out.find("use anyhow").unwrap();
    assert!(
        std_io < fence && fence < anyhow_pos,
        "fence must keep std imports above the divider:\n{out}"
    );
    // Within each section, order is preserved or canonicalized.
    let std_fmt = out.find("use std::fmt").unwrap();
    assert!(
        std_fmt < std_io,
        "std::fmt before std::io within section:\n{out}"
    );
    assert_idempotent(&out);
}

#[test]
fn fence_does_not_eat_blank_lines() {
    // The visual gap (one blank above + one blank below the comment)
    // must be preserved in the output.
    let input = "\
fn a() {}

// fence

fn b() {}
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    assert!(
        out.contains("\n\n// fence\n\n"),
        "fence must keep blank lines on both sides:\n{out:?}"
    );
}
