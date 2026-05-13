//! Coverage for floating-comment handling: by default a `//`-line comment
//! block sandwiched between blank lines on both sides is *attached* to the
//! item below it and moves with that item. With `--no-floating-comment-attach`
//! the comment instead becomes a fixed section divider (fence) — items above
//! and below cannot reorder across it. Doc comments (`///`, `//!`) are
//! excluded from both behaviours.

use cargo_reorder::{Config, reorder_source, reorder_source_with};

fn fence_cfg() -> Config {
    Config {
        no_floating_comment_attach: true,
        ..Config::default()
    }
}

fn assert_idempotent_with(out: &str, cfg: &Config) {
    let again = reorder_source_with(out, cfg).unwrap();
    assert_eq!(
        out, again,
        "not idempotent:\nfirst:\n{out}\nsecond:\n{again}"
    );
}

// ── default attach-mode tests ──────────────────────────────────────────

#[test]
fn floating_comment_moves_with_next_item() {
    // const (weight 40) sorts before struct (weight 51). The comment,
    // originally between them, should attach to const and move up with it.
    let input = "\
struct S;

// section divider

const C: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let comment_pos = out.find("// section divider").unwrap();
    let const_pos = out.find("const C").unwrap();
    let struct_pos = out.find("struct S").unwrap();
    assert!(
        comment_pos < const_pos && const_pos < struct_pos,
        "comment must move with const above struct:\n{out}"
    );
}

#[test]
fn floating_comment_attaches_to_next_item_by_prefix_sort() {
    // Struct with prefix "Very" (mean len 12) vs "X" (mean len 1).
    // "X" sorts first, and the comment attaches to X, moving up with it.
    let input = "\
struct VeryLongName;

// section marker

struct X;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let comment_pos = out.find("// section marker").unwrap();
    let x_pos = out.find("struct X").unwrap();
    let vl_pos = out.find("struct VeryLongName").unwrap();
    assert!(
        comment_pos < x_pos && x_pos < vl_pos,
        "comment must move with struct X to the top:\n{out}"
    );
}

#[test]
fn multiple_floating_comments_each_attach_to_their_item() {
    // const → struct ordering (category weights 40 vs 51).
    // Each comment stays glued to its item.
    let input = "\
struct B;

// group 1

const A: u32 = 1;

// group 2

struct C;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // After sorting: const A first, then struct B, struct C.
    let g1 = out.find("// group 1").unwrap();
    let a = out.find("const A").unwrap();
    let g2 = out.find("// group 2").unwrap();
    let b = out.find("struct B").unwrap();
    let c = out.find("struct C").unwrap();
    assert!(
        g1 < a && a < b && b < g2 && g2 < c,
        "each comment must attach to its item:\n{out}"
    );
}

#[test]
fn comment_without_surrounding_blanks_is_leading_trivia() {
    // Comment immediately above an item (no blank between) is the
    // item's leading trivia — not a floating comment.
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
fn doc_comment_is_not_a_floating_comment() {
    // `///` is an outer doc comment; syn associates it with the next
    // item as an attribute, so it should never even reach our fence
    // detector. The next item must still be allowed to move.
    let input = "\
struct B;

/// docs for next
struct A;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let docs_pos = out.find("/// docs for next").unwrap();
    let a_pos = out.find("struct A").unwrap();
    let between = &out[docs_pos..a_pos];
    assert!(
        !between.contains("struct"),
        "doc must stay immediately above its struct:\n{out}"
    );
}

// ── fence-mode tests (--no-floating-comment-attach) ────────────────────

#[test]
fn fence_keeps_items_above_above() {
    let cfg = fence_cfg();
    let input = "\
extern crate test;
mod support;

// TODO: rewrite this section

use std::fmt;
fn z() {}
";
    let out = reorder_source_with(input, &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let mod_pos = out.find("mod support;").unwrap();
    let todo_pos = out.find("// TODO").unwrap();
    let use_pos = out.find("use std::fmt").unwrap();
    assert!(
        mod_pos < todo_pos && todo_pos < use_pos,
        "mod must stay above the fence and use must stay below:\n{out}"
    );
    assert_idempotent_with(&out, &cfg);
}

#[test]
fn fence_keeps_items_below_below() {
    let cfg = fence_cfg();
    let input = "\
fn first_above() {}

// === section break ===

use std::fmt;
struct S;
";
    let out = reorder_source_with(input, &cfg).unwrap();
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
    assert_idempotent_with(&out, &cfg);
}

#[test]
fn fence_does_not_eat_blank_lines() {
    // The visual gap (one blank above + one blank below the comment)
    // must be preserved in the output.
    let cfg = fence_cfg();
    let input = "\
fn a() {}

// fence

fn b() {}
";
    let out = reorder_source_with(input, &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    assert!(
        out.contains("\n\n// fence\n\n"),
        "fence must keep blank lines on both sides:\n{out:?}"
    );
}

#[test]
fn fence_inside_inline_mod_when_recursed() {
    let cfg = fence_cfg();
    let input = "\
mod inner {
    fn a() {}

    // fence inside

    use std::fmt;
    fn b() {}
}
";
    let out = reorder_source_with(input, &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let a_pos = out.find("fn a").unwrap();
    let fence_pos = out.find("// fence inside").unwrap();
    let use_pos = out.find("use std::fmt").unwrap();
    assert!(
        a_pos < fence_pos && fence_pos < use_pos,
        "inline-mod fence semantics not preserved:\n{out}"
    );
    assert_idempotent_with(&out, &cfg);
}

#[test]
fn fence_between_two_use_blocks_is_preserved() {
    let cfg = fence_cfg();
    let input = "\
use std::fmt;
use std::io;

// third-party imports below

use anyhow::Result;
use serde::Serialize;

fn x() {}
";
    let out = reorder_source_with(input, &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let std_io = out.find("use std::io").unwrap();
    let fence = out.find("third-party").unwrap();
    let anyhow_pos = out.find("use anyhow").unwrap();
    assert!(
        std_io < fence && fence < anyhow_pos,
        "fence must keep std imports above the divider:\n{out}"
    );
    let std_fmt = out.find("use std::fmt").unwrap();
    assert!(
        std_fmt < std_io,
        "std::fmt before std::io within section:\n{out}"
    );
    assert_idempotent_with(&out, &cfg);
}

#[test]
fn fence_with_explicit_recurse_off_still_works_at_top_level() {
    let cfg = Config {
        no_inline_mods: true,
        ..fence_cfg()
    };
    let input = "\
fn upper() {}

// fence

use std::fmt;
";
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
fn multi_line_comment_block_acts_as_one_fence() {
    let cfg = fence_cfg();
    let input = "\
fn a() {}

// section header line 1
// section header line 2
// section header line 3

use std::fmt;
fn b() {}
";
    let out = reorder_source_with(input, &cfg).unwrap();
    syn::parse_file(&out).unwrap();
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
    assert_idempotent_with(&out, &cfg);
}

#[test]
fn multiple_fences_partition_into_three_sections() {
    let cfg = fence_cfg();
    let input = "\
fn upper_a() {}
fn upper_b() {}

// --- section 1 ---

fn middle() {}

// --- section 2 ---

use std::fmt;
fn lower() {}
";
    let out = reorder_source_with(input, &cfg).unwrap();
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
    assert_idempotent_with(&out, &cfg);
}

// ── block comment (/* */) tests ────────────────────────────────────────

#[test]
fn block_comment_attaches_to_next_item() {
    // Single-line `/* */` is a floating comment and attaches by default.
    let input = "\
struct S;

/* section divider */

const C: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let comment_pos = out.find("/* section divider */").unwrap();
    let const_pos = out.find("const C").unwrap();
    let struct_pos = out.find("struct S").unwrap();
    assert!(
        comment_pos < const_pos && const_pos < struct_pos,
        "block comment must move with const above struct:\n{out}"
    );
}

#[test]
fn multi_line_block_comment_attaches() {
    let input = "\
struct S;

/* multi-line
   section divider */

const C: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let comment_pos = out.find("/* multi-line").unwrap();
    let const_pos = out.find("const C").unwrap();
    let struct_pos = out.find("struct S").unwrap();
    assert!(
        comment_pos < const_pos && const_pos < struct_pos,
        "multi-line block comment must move with const:\n{out}"
    );
}

#[test]
fn block_comment_fence_mode() {
    let cfg = fence_cfg();
    let input = "\
fn upper() {}

/* === fence === */

use std::fmt;
";
    let out = reorder_source_with(input, &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let upper = out.find("fn upper").unwrap();
    let fence = out.find("/* === fence === */").unwrap();
    let use_pos = out.find("use std::fmt").unwrap();
    assert!(
        upper < fence && fence < use_pos,
        "block comment fence broken:\n{out}"
    );
}

#[test]
fn multi_line_block_comment_fence_mode() {
    let cfg = fence_cfg();
    let input = "\
fn upper() {}

/* section
 * header
 */

use std::fmt;
";
    let out = reorder_source_with(input, &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let upper = out.find("fn upper").unwrap();
    let fence = out.find("/* section").unwrap();
    let use_pos = out.find("use std::fmt").unwrap();
    assert!(
        upper < fence && fence < use_pos,
        "multi-line block comment fence broken:\n{out}"
    );
}

#[test]
fn block_comment_mixed_with_line_comment_attaches() {
    // `/* */` and `//` interleaved → one comment block, attaches together.
    let input = "\
struct S;

/* start */
// middle
/* end */

const C: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let start = out.find("/* start */").unwrap();
    let middle = out.find("// middle").unwrap();
    let end = out.find("/* end */").unwrap();
    let const_pos = out.find("const C").unwrap();
    assert!(
        start < middle && middle < end && end < const_pos,
        "all comments must stay together above const:\n{out}"
    );
}

#[test]
fn block_comment_with_line_comments_inside_fence_mode() {
    // A multi-line `/* */` containing `//`-style text → treated as fence.
    let cfg = fence_cfg();
    let input = "\
fn upper() {}

/* section
// subsection A
// subsection B
 */

use std::fmt;
";
    let out = reorder_source_with(input, &cfg).unwrap();
    syn::parse_file(&out).unwrap();
    let upper = out.find("fn upper").unwrap();
    let fence = out.find("/* section").unwrap();
    let use_pos = out.find("use std::fmt").unwrap();
    assert!(
        upper < fence && fence < use_pos,
        "block comment containing // must still act as fence:\n{out}"
    );
    assert_idempotent_with(&out, &cfg);
}

#[test]
fn block_comment_with_star_slash_on_own_line_attaches() {
    // Common multi-line style: `*/` sits on its own line.
    let input = "\
struct S;

/*
 * section divider
 */

const C: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let comment_pos = out.find("/*").unwrap();
    let const_pos = out.find("const C").unwrap();
    let struct_pos = out.find("struct S").unwrap();
    assert!(
        comment_pos < const_pos && const_pos < struct_pos,
        "block comment with */ on own line must move with const:\n{out}"
    );
}

#[test]
fn block_comment_with_leading_whitespace_attaches() {
    // Indented block comment still detected and attached.
    let input = "\
struct S;

    /* indented divider */

const C: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let comment_pos = out.find("/* indented divider */").unwrap();
    let const_pos = out.find("const C").unwrap();
    let struct_pos = out.find("struct S").unwrap();
    assert!(
        comment_pos < const_pos && const_pos < struct_pos,
        "indented block comment must move with const:\n{out}"
    );
}

#[test]
fn block_comment_empty_attaches() {
    // Empty `/**/` is still a valid floating comment.
    let input = "\
struct S;

/**/

const C: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    let comment_pos = out.find("/**/").unwrap();
    let const_pos = out.find("const C").unwrap();
    let struct_pos = out.find("struct S").unwrap();
    assert!(
        comment_pos < const_pos && const_pos < struct_pos,
        "empty block comment must move with const:\n{out}"
    );
}

#[test]
fn block_comment_not_floating_without_leading_blank() {
    // `/* ... */` directly after an item (no blank above) with a blank
    // before the next item → trailing trivia of the item above, not a
    // floating comment (which needs blanks on *both* sides).
    let input = "\
struct S;
/* trailing comment */

const C: u32 = 1;
";
    let out = reorder_source(input).unwrap();
    syn::parse_file(&out).unwrap();
    // const (40) sorts before struct (51), but the comment stays with
    // struct (its anchor item).
    let comment_pos = out.find("/* trailing comment */").unwrap();
    let struct_pos = out.find("struct S").unwrap();
    assert!(
        struct_pos < comment_pos,
        "comment without leading blank is previous item's trailing trivia:\n{out}"
    );
}
