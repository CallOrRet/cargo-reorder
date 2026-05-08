//! Stitch sorted [`Block`]s back into a source string.
//!
//! The emit phase decides where to insert blank lines:
//!
//!   * **Import boundaries** (default, off only with
//!     `cfg.no_import_groups`): a blank line goes between consecutive
//!     import items in different `(category, visual)` buckets, and
//!     between the last import and the first non-import item.
//!     `crate` / `super` / `self` / `local-mod` share one
//!     [`VisualGroup`] so they stay glued together.
//!
//!   * **`pub mod` / `mod` boundary** (the default pub-mod-first
//!     grouping): a blank line between the two `Mod` sub-buckets.
//!
//! Blank lines that already exist in source trivia (a `Block`'s
//! `leading` / `trailing`) are respected — we never insert a duplicate.

use crate::imports::ImportGroup;
use crate::reorder::{Block, Category, Config};
use crate::text::{ends_with_blank_line, starts_with_blank_line};

pub(crate) fn assemble(
    header: &str,
    header_extra: &str,
    blocks: &[Block],
    footer: &str,
    cfg: &Config,
) -> String {
    let mut out = String::new();
    out.push_str(header);
    out.push_str(header_extra);

    let mut prev: Option<&Block> = None;
    for blk in blocks {
        if let Some(p) = prev {
            if needs_blank_separator(p, blk, cfg) && !out.ends_with("\n\n") {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
            }
        }
        out.push_str(&blk.leading);
        out.push_str(&blk.body);
        out.push_str(&blk.trailing);
        prev = Some(blk);
    }

    if footer.is_empty() {
        while out.ends_with("\n\n") {
            out.pop();
        }
    }
    out.push_str(footer);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn is_import_like(c: Category) -> bool {
    matches!(c, Category::ExternCrate | Category::Use | Category::PubUse)
}
fn import_boundary(p: &Block, blk: &Block) -> bool {
    let prev_visual = p.import_group.map(ImportGroup::visual);
    let blk_visual = blk.import_group.map(ImportGroup::visual);
    let prev_is_import = is_import_like(p.category);
    let blk_is_import = is_import_like(blk.category);
    if prev_is_import && blk_is_import {
        p.category != blk.category || prev_visual != blk_visual
    } else {
        prev_is_import != blk_is_import
    }
}

fn pub_mod_boundary(p: &Block, blk: &Block) -> bool {
    p.category == Category::Mod && blk.category == Category::Mod && p.mod_is_pub != blk.mod_is_pub
}

fn needs_blank_separator(p: &Block, blk: &Block, cfg: &Config) -> bool {
    let already_blank = ends_with_blank_line(&p.trailing) || starts_with_blank_line(&blk.leading);
    if already_blank {
        return false;
    }
    if !cfg.no_import_groups && import_boundary(p, blk) {
        return true;
    }
    if !cfg.no_pub_mod_first && pub_mod_boundary(p, blk) {
        return true;
    }
    false
}
