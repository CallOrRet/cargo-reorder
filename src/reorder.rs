//! Reordering pipeline: parse with `syn`, slice the source into per-item
//! `Block`s (leading comments + body + trailing gap), assign sort keys
//! (with bare top-level macro invocations sitting on private "barrier"
//! segments so siblings can't reorder past them), sort, reassemble.

use std::collections::{HashMap, HashSet};
use std::fmt;

use syn::{File, Item};

use crate::fields::GroupSortKey;
use crate::imports::ImportGroup;
use crate::text::{extract_floating_comment, split_at_last_blank, split_keep_endings, take_lines};

/// Traits auto-imported by the Rust compiler via the std/core prelude —
/// the union of v1 (2015/2018), rust_2021, and rust_2024 entries.
/// Authoritative source:
/// https://github.com/rust-lang/rust/blob/master/library/std/src/prelude/mod.rs
const PRELUDE_TRAITS: &[&str] = &[
    // marker (5)
    "Send",
    "Sync",
    "Sized",
    "Unpin",
    "Copy",
    // ops (7) — Fn family + AsyncFn family + Drop
    "Fn",
    "FnMut",
    "FnOnce",
    "AsyncFn",
    "AsyncFnMut",
    "AsyncFnOnce",
    "Drop",
    // basic (3)
    "Clone",
    "Default",
    "ToOwned",
    // cmp (4)
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    // conversion (6) — TryFrom/TryInto added in rust_2021
    "From",
    "Into",
    "TryFrom",
    "TryInto",
    "AsRef",
    "AsMut",
    // iter (6) — FromIterator added in rust_2021
    "Iterator",
    "IntoIterator",
    "FromIterator",
    "Extend",
    "DoubleEndedIterator",
    "ExactSizeIterator",
    // string (1)
    "ToString",
    // future (2) — added in rust_2024
    "Future",
    "IntoFuture",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Category {
    ExternCrate,
    Use,
    PubUse,
    Mod,
    Const,
    Static,
    TypeAlias,
    Enum,
    Struct,
    Trait,
    Impl,
    ForeignMod,
    Fn,
    AsyncFn,
    Macro,
    TestMod,
    Other,
    /// Synthetic block representing a "floating" comment block in the
    /// source — a `//`-line comment surrounded by blank lines on both
    /// sides. The comment text is stored in the block's `leading`; the
    /// block has empty body and trailing. The sort_key pins it on its
    /// own private segment so neighbouring items can't reorder across
    /// it (it acts like a section divider).
    Fence,
}

impl Category {
    /// Sort weight. Default mod-first; `no_mod_before_use` flips to use-first.
    fn weight(self, cfg: &Config) -> u32 {
        let use_first = cfg.no_mod_before_use;
        match self {
            Category::ExternCrate => 0,
            Category::Mod => {
                if use_first {
                    30
                } else {
                    10
                }
            }
            Category::Use => {
                if use_first {
                    11
                } else {
                    31
                }
            }
            Category::PubUse => {
                if use_first {
                    12
                } else {
                    32
                }
            }
            Category::Const => 40,
            Category::Static => 41,
            Category::TypeAlias => 42,
            Category::Enum => 50,
            Category::Struct => 51,
            Category::Trait => {
                if cfg.no_trait_before_struct {
                    60
                } else {
                    49
                }
            }
            Category::Impl => 70,
            Category::ForeignMod => 80,
            Category::Fn => 90,
            Category::AsyncFn => 91,
            Category::Macro => 92,
            Category::TestMod => 999,
            Category::Other => 500,
            // Fence sits alone on its own segment (the `segment` field of
            // its SortKey separates it from any item), so this `primary`
            // weight is never compared against another block's. Pick a
            // neutral value.
            Category::Fence => 0,
        }
    }

    fn classify(item: &Item) -> Self {
        match item {
            Item::ExternCrate(_) => Category::ExternCrate,
            Item::Use(u) => match u.vis {
                syn::Visibility::Inherited => Category::Use,
                _ => Category::PubUse,
            },
            Item::Mod(m) => {
                if has_cfg_test(&m.attrs) {
                    Category::TestMod
                } else {
                    Category::Mod
                }
            }
            Item::Const(_) => Category::Const,
            Item::Static(_) => Category::Static,
            Item::Type(_) => Category::TypeAlias,
            Item::Enum(_) => Category::Enum,
            Item::Struct(_) | Item::Union(_) => Category::Struct,
            Item::Trait(_) | Item::TraitAlias(_) => Category::Trait,
            Item::Impl(_) => Category::Impl,
            Item::ForeignMod(_) => Category::ForeignMod,
            Item::Fn(f) => {
                if f.sig.asyncness.is_some() {
                    Category::AsyncFn
                } else {
                    Category::Fn
                }
            }
            Item::Macro(_) => Category::Macro,
            _ => Category::Other,
        }
    }
}

/// How an `impl` block is classified relative to a target type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ImplKind {
    Inherent = 0,
    StdTrait = 1,
    CrateTrait = 2,
    ExternalTrait = 3,
}

#[derive(Debug)]
pub enum ReorderError {
    Parse(syn::Error),
}

impl From<syn::Error> for ReorderError {
    fn from(e: syn::Error) -> Self {
        ReorderError::Parse(e)
    }
}

impl std::error::Error for ReorderError {}

impl fmt::Display for ReorderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReorderError::Parse(e) => write!(f, "failed to parse Rust source: {e}"),
        }
    }
}

/// Use-tree origin tag — used by both std-import and crate-import scans.
#[derive(Copy, Clone, PartialEq, Eq)]
enum ImportOrigin {
    Std,
    Crate,
    Unknown,
}

pub(crate) struct Block {
    pub(crate) body: String,
    pub(crate) leading: String,
    pub(crate) category: Category,
    pub(crate) trailing: String,
    pub(crate) sort_key: SortKey,
    /// Visibility for `mod` items, used by the blank-line logic for
    /// the default pub-mod-first grouping.
    /// `Some(true)` = pub mod, `Some(false)` = private mod,
    /// `None` = not a mod.
    pub(crate) mod_is_pub: Option<bool>,
    pub(crate) import_group: Option<ImportGroup>,
}

/// User-tunable behaviour. Every field defaults to `false`; opting *in*
/// to a non-default behaviour is always `field = true`. The CLI maps each
/// field one-to-one to a `--<field-name>` flag (no `ArgAction::SetFalse`
/// indirection). See README for the rationale on each flag.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Enable reordering function parameter lists. This is off by
    /// default because parameter order is part of a function's call
    /// contract. When on, the first receiver parameter (`self`,
    /// `mut self`, `&self`, `&mut self`) stays first and the remaining
    /// ordinary identifier parameters use the field grouping rule.
    pub fn_args: bool,
    /// Disable reordering named fields inside `struct` / `union` /
    /// `enum` (and inside enum variants). When off, fields are grouped by
    /// their snake_case / PascalCase / camelCase first word, within each
    /// group sorted shortest-name-first, and the groups are emitted in
    /// ascending order of the group's mean name length. ABI- and
    /// semantics-affecting shapes are always skipped: any
    /// `#[repr(...)]`, any `#[derive(PartialOrd | Ord)]`, enums whose
    /// any variant carries an explicit discriminant, enum variant-order
    /// sorting when any variant is unit-like, and tuple/unit variants.
    pub no_fields: bool,
    /// Disable the prefix-group + length sort applied **inside `impl`
    /// and `trait` bodies** (the const → type → fn → async fn category
    /// order, plus the within-category prefix sort). When on, the body
    /// of every `impl` / `trait` is left in the user's source order.
    /// Field-level and top-level grouping are unaffected — those stay
    /// under `no_fields`.
    pub no_impl_fns: bool,
    /// Disable forcing `#[cfg(test)] mod ...` to the end of the file.
    pub no_tests_last: bool,
    /// Disable recursing into inline `mod foo { ... }` blocks.
    /// See README for the skip list (test mods, `#[macro_use]` mods,
    /// pure-`use` mods) — those are always skipped regardless.
    pub no_inline_mods: bool,
    /// Disable anchoring `impl` blocks to their target type
    /// (inherent → std trait → external trait).
    pub no_impl_grouping: bool,
    /// Disable splitting imports into std / external / crate-local groups.
    pub no_import_groups: bool,
    /// Preserve the source order of `pub mod` / `mod` instead of the
    /// default pub-mod-first grouping.
    pub no_pub_mod_first: bool,
    /// Disable putting `mod foo;` before `use ...;`. Default is
    /// mod-first — matches the majority pattern in our 21-project
    /// sample (12/21 mod-first vs 7/21 use-first); see README.
    pub no_mod_before_use: bool,
    /// Disable ordering shorter trait paths first
    /// (`impl Debug for Foo` before `impl std::fmt::Debug for Foo`).
    pub no_short_trait_first: bool,
    /// Disable trimming existing blank lines between reordered
    /// field-like entries. By default, multi-line field sorting removes
    /// blank lines between fields; with this on, original blank lines
    /// move with the following field. Blank lines before the first
    /// emitted field are still removed.
    pub no_trim_field_blanks: bool,
    /// Disable reordering same-line field-like lists: single-line
    /// `struct` / `union` / `enum` definitions, struct-literal
    /// expressions. By default these entries are permuted in-place and
    /// the output stays single-line.
    pub no_single_line_fields: bool,
    /// Disable putting `trait` ahead of `enum` / `struct` / `union`.
    /// Default is trait-first — matches the majority in our sample
    /// (14/20 trait-first; see README).
    pub no_trait_before_struct: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SortKey {
    /// Items in different segments never cross each other. Bare top-level
    /// macro invocations get a private odd segment (pinned in place);
    /// everything else lives in the surrounding even segments.
    segment: u32,
    /// Primary category weight.
    primary: u32,
    /// Same-category grouping key: exact group mean, group source order,
    /// name length, and source index.
    /// Items in a category-eligible-for-grouping
    /// (struct/union/enum/trait/fn/async-fn) get a key derived from their
    /// name's prefix-group; impls inherit their target type's
    /// key (so impls follow the type to its new sorted position).
    /// Items not subject to grouping get a source-order key.
    anchor: (GroupSortKey, usize),
    /// 0 = the type definition itself, 1 = a follower impl.
    follower: u8,
    /// Within a type's followers: inherent (0) → std trait (1) →
    /// crate trait (2) → external trait (3).
    impl_kind: u8,
    /// Trait-path segment count for trait `impl` blocks (0 for non-impls
    /// and inherent impls). Shorter paths sort first when
    /// `short_trait_first` is on; otherwise this is always 0 and
    /// the field is inert.
    trait_path_len: u8,
    /// Sub-bucket inside a category. For `use` items it encodes the
    /// std → external → crate/super/self/local-mod visual group.
    /// For `mod` items in the default pub-mod-first mode it's reused
    /// as the pub-vs-private subgroup (0 = pub mod, 1 = private mod).
    /// Otherwise 0.
    import_group: u8,
    /// Tie-breaker: original source index (preserves stable order).
    pub(crate) original_index: usize,
}

/// Per-scope reference data shared across every call to
/// [`compute_sort_key`]: name → (idx, category) lookup for impl
/// anchoring, plus the three trait-classification name sets.
struct ScopeIndex<'a> {
    name_index: &'a HashMap<String, (usize, Category)>,
    /// Per-item group key for items in
    /// group-eligible top-level categories
    /// (struct/union/enum/trait/fn/async-fn). Items not in the map
    /// fall back to source order.
    group_keys: &'a HashMap<usize, GroupSortKey>,
    std_imports: &'a HashSet<String>,
    local_traits: &'a HashSet<String>,
    crate_imports: &'a HashSet<String>,
}

fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if !a.path().is_ident("cfg") {
            return false;
        }
        let mut found = false;
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("test") {
                found = true;
            }
            Ok(())
        });
        found
    })
}

/// Last segment of a `Type` path — the name an `impl` block targets.
fn type_last_segment(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(tp) => tp.path.segments.last().map(|s| s.ident.to_string()),
        syn::Type::Reference(r) => type_last_segment(&r.elem),
        syn::Type::Paren(p) => type_last_segment(&p.elem),
        syn::Type::Group(g) => type_last_segment(&g.elem),
        _ => None,
    }
}

/// Internal entry point. Recurses into itself for cargo-script
/// frontmatter stripping and (when `!cfg.no_inline_mods` is on)
/// for inline `mod foo { ... }` body recursion.
fn reorder_inner(
    source: &str,
    cfg: &Config,
    source_path: Option<&std::path::Path>,
) -> Result<String, ReorderError> {
    // Cargo-script: strip frontmatter, reorder body, stitch back.
    if let Some((prefix, body)) = crate::frontmatter::split(source) {
        let reordered = reorder_inner(&body, cfg, source_path)?;
        return Ok(format!("{prefix}{reordered}"));
    }

    // Parse once up front, then optionally rewrite inline mod bodies
    // recursively. When the inline-mod pass changes the source we have
    // to re-parse the rewritten string; when it doesn't (the common
    // case for files without inline mods, or with only skip-listed
    // ones), we hand the original AST straight to the main pipeline.
    let mut parsed: File = syn::parse_file(source)?;
    if parsed.items.is_empty() {
        return Ok(source.to_string());
    }

    let owned: Option<String> = if !cfg.no_inline_mods {
        let new_source = recurse_inline_mods(source, &parsed, cfg)?;
        if let Some(new_src) = new_source {
            parsed = syn::parse_file(&new_src)?;
            Some(new_src)
        } else {
            None
        }
    } else {
        None
    };
    let source: &str = owned.as_deref().unwrap_or(source);

    let lines: Vec<&str> = split_keep_endings(source);
    let header_end_line = compute_header_end_line(&parsed);
    let ranges: Vec<(usize, usize)> = parsed
        .items
        .iter()
        .map(|item| {
            use syn::spanned::Spanned;
            let span = item.span();
            let s = span.start().line.max(1);
            let e = span.end().line.max(s);
            (s, e)
        })
        .collect();

    // Anchor index for `impl` blocks that target a local type/trait.
    let mut name_index: HashMap<String, (usize, Category)> = HashMap::new();
    // Local `mod foo;` names — used to classify `use foo::...` as local-mod.
    let mut local_mods: HashSet<String> = HashSet::new();
    // Names imported into local scope (possibly under an `as` rename).
    // `std_imports` holds names whose origin is std/core/alloc; the
    // classifier uses these to mark single-segment trait names as
    // std-trait. `crate_imports` does the same for crate/self/super, so
    // `use crate::MyTrait as M;` makes `impl M for Foo` a crate-trait.
    let mut std_imports: HashSet<String> = HashSet::new();
    let mut crate_imports: HashSet<String> = HashSet::new();
    // Trait names declared at the top of this file — `impl LocalTrait
    // for X` should classify as crate-trait, not external.
    let mut local_traits: HashSet<String> = HashSet::new();
    // Per-category names for top-level group-sorting. We collect
    // `(idx, name)` pairs bucketed by `Category`, then run
    // `crate::fields::compute_group_keys` per bucket so each item gets a
    // group key derived only from same-category siblings.
    let mut category_names: HashMap<Category, Vec<(usize, String)>> = HashMap::new();
    for (idx, item) in parsed.items.iter().enumerate() {
        let cat = Category::classify(item);
        let mut record = |name: String| {
            category_names
                .entry(cat)
                .or_default()
                .push((idx, name.clone()));
            name_index.insert(name, (idx, cat));
        };
        match item {
            Item::Struct(s) => record(s.ident.to_string()),
            Item::Enum(e) => record(e.ident.to_string()),
            Item::Union(u) => record(u.ident.to_string()),
            Item::Trait(t) => {
                local_traits.insert(t.ident.to_string());
                record(t.ident.to_string());
            }
            Item::TraitAlias(t) => {
                local_traits.insert(t.ident.to_string());
                record(t.ident.to_string());
            }
            Item::Fn(f) => {
                category_names
                    .entry(cat)
                    .or_default()
                    .push((idx, f.sig.ident.to_string()));
            }
            Item::Mod(m) => {
                local_mods.insert(m.ident.to_string());
            }
            Item::Use(u) => collect_use_imports(
                &u.tree,
                ImportOrigin::Unknown,
                &mut std_imports,
                &mut crate_imports,
            ),
            _ => {}
        }
    }
    // Build per-item group keys. Only categories the user wants
    // grouped get a key; other items fall back to source order.
    let mut group_keys: HashMap<usize, GroupSortKey> = HashMap::new();
    if !cfg.no_fields {
        for (cat, names) in &category_names {
            if !is_top_level_groupable(*cat) {
                continue;
            }
            let keys = crate::fields::compute_group_keys(
                names.iter().map(|(idx, name)| (*idx, name.as_str())),
            );
            group_keys.extend(keys);
        }
    }

    // Macro placement: every `macro_rules!`, every bare top-level
    // macro invocation, and every `#[macro_use] mod` becomes a hard
    // barrier — pinned in source position, with no other item allowed
    // to reorder across it. This is a deliberate trade of sort
    // quality for a very simple correctness story; see `src/macros.rs`
    // for the rationale.
    let segments = crate::macros::compute_segments(&parsed.items);

    // Floating-comment fences: a `//`-line comment block sandwiched by
    // blank lines on both sides is treated as a section divider. The
    // comment becomes a synthetic Block pinned on its own segment so
    // items above can't reorder past items below. Per-fence we widen
    // the segment numbering by FENCE_STRIDE so the fence has room to
    // sit strictly between the items it separates.
    const FENCE_STRIDE: u32 = 100;
    let mut bumped_segments: Vec<u32> = segments
        .iter()
        .map(|s| s.saturating_mul(FENCE_STRIDE))
        .collect();
    let mut fence_after: Vec<Option<(String, String)>> = vec![None; parsed.items.len()];
    let mut fence_bump: u32 = 0;
    for i in 0..parsed.items.len() {
        bumped_segments[i] = bumped_segments[i].saturating_add(fence_bump);
        if i + 1 < parsed.items.len() {
            let (_, end_i) = ranges[i];
            let (start_next, _) = ranges[i + 1];
            let gap = take_lines(&lines, end_i + 1, start_next.saturating_sub(1));
            if let Some((comment, residual)) = extract_floating_comment(&gap) {
                fence_after[i] = Some((comment, residual));
                fence_bump = fence_bump.saturating_add(FENCE_STRIDE);
            }
        }
    }
    let segments = bumped_segments;

    // Pre-item gap (before first item).
    let pre_first_gap = take_lines(&lines, header_end_line + 1, ranges[0].0.saturating_sub(1));
    let (header_extra, first_leading) = split_at_last_blank(&pre_first_gap);

    let last_end = ranges.last().unwrap().1;
    let footer = take_lines(&lines, last_end + 1, lines.len());

    let scope = ScopeIndex {
        name_index: &name_index,
        group_keys: &group_keys,
        std_imports: &std_imports,
        local_traits: &local_traits,
        crate_imports: &crate_imports,
    };
    let mut blocks: Vec<Block> = Vec::with_capacity(parsed.items.len());
    for (idx, (item, &(start, end))) in parsed.items.iter().zip(ranges.iter()).enumerate() {
        let leading = if idx == 0 {
            first_leading.clone()
        } else {
            String::new()
        };
        let body = take_lines(&lines, start, end);
        let body = if cfg.no_fields {
            body
        } else if cfg.no_impl_fns && matches!(item, Item::Impl(_) | Item::Trait(_)) {
            // `--no-impl-fns`: keep impl/trait bodies in
            // source order. Field-level (struct/union/enum) reorder
            // still runs — that's a separate pass.
            body
        } else {
            crate::fields::reorder_in_item(item, &body, start, cfg.no_trim_field_blanks)
                .unwrap_or(body)
        };
        let body = if cfg.no_fields {
            body
        } else {
            crate::fields::reorder_expr_structs_in_item_text(&body, cfg.no_trim_field_blanks)
                .unwrap_or(body)
        };
        let body = if cfg.no_fields || cfg.no_single_line_fields {
            body
        } else {
            crate::fields::reorder_single_line_lists_in_item_text(&body).unwrap_or(body)
        };
        let body = if cfg.fn_args {
            crate::fields::reorder_fn_args_in_item_text(&body).unwrap_or(body)
        } else {
            body
        };
        let category = Category::classify(item);
        let import_group = match item {
            Item::Use(u) => Some(ImportGroup::classify(u, &local_mods)),
            _ => None,
        };
        let mod_is_pub = match item {
            Item::Mod(m) if !has_cfg_test(&m.attrs) => {
                Some(!matches!(m.vis, syn::Visibility::Inherited))
            }
            _ => None,
        };
        // segments[idx] is already widened by FENCE_STRIDE and bumped
        // past any preceding fences.
        let sort_key = compute_sort_key(
            item,
            idx,
            segments[idx],
            category,
            import_group,
            mod_is_pub,
            cfg,
            &scope,
        );
        blocks.push(Block {
            body,
            leading,
            category,
            trailing: String::new(),
            sort_key,
            mod_is_pub,
            import_group,
        });
    }

    for i in 0..parsed.items.len().saturating_sub(1) {
        let (_, end_i) = ranges[i];
        let (start_next, _) = ranges[i + 1];
        let gap = take_lines(&lines, end_i + 1, start_next.saturating_sub(1));
        // If a floating-comment fence lives in this gap, split on what
        // remains *after* extracting the comment text — the comment
        // itself is owned by the fence block we append below.
        let effective_gap = match &fence_after[i] {
            Some((_, residual)) => residual.clone(),
            None => gap,
        };
        let (trailing, next_leading) = split_at_last_blank(&effective_gap);
        blocks[i].trailing = trailing;
        blocks[i + 1].leading = next_leading;
    }

    // Append fence blocks. Each one sits on `bumped_segments[i] +
    // FENCE_STRIDE/2`, which is strictly between the preceding item's
    // (post-bump) segment and the next item's (already further bumped)
    // segment — so sort_by_key keeps the fence wedged between them
    // regardless of how items shuffle within their own segments.
    // `original_index = n_items + fence_idx` parks fences just past
    // the real items so the stable-sort tiebreaker keeps them next to
    // their original neighbours, without colliding with any real
    // item's [0, n_items) index range.
    let n_items = parsed.items.len();
    let mut fence_idx = 0usize;
    for (i, fence) in fence_after.iter().enumerate() {
        if let Some((comment_text, _)) = fence {
            let fence_segment = segments[i].saturating_add(FENCE_STRIDE / 2);
            blocks.push(Block {
                body: String::new(),
                leading: comment_text.clone(),
                category: Category::Fence,
                trailing: String::new(),
                sort_key: SortKey {
                    anchor: (GroupSortKey::source_order(n_items + fence_idx), 0),
                    segment: fence_segment,
                    primary: 0,
                    follower: 0,
                    impl_kind: 0,
                    import_group: 0,
                    trait_path_len: 0,
                    original_index: n_items + fence_idx,
                },
                mod_is_pub: None,
                import_group: None,
            });
            fence_idx += 1;
        }
    }

    blocks.sort_by_key(|b| b.sort_key);

    let header = take_lines(&lines, 1, header_end_line);
    let assembled = crate::emit::assemble(&header, &header_extra, &blocks, &footer, cfg);
    // Self-check: if the rewritten source no longer parses (a structural
    // bug like an impl-block boundary or a `/* ... */` block comment in
    // the gap between items got scrambled), fall back to the input
    // rather than emit syntactically broken Rust. The file is left
    // untouched and we surface a one-line warning to stderr so the
    // event isn't silent under `--check`.
    if assembled != source {
        if let Err(e) = syn::parse_file(&assembled) {
            let where_ = source_path
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<source>".to_string());
            eprintln!(
                "cargo-reorder: skipping {where_}: rewrite produced invalid Rust ({e}); file left unchanged"
            );
            return Ok(source.to_string());
        }
    }
    Ok(assembled)
}

/// Reorder with the default [`Config`].
pub fn reorder_source(source: &str) -> Result<String, ReorderError> {
    reorder_source_with_path(source, None, &Config::default())
}

pub fn reorder_source_with(source: &str, cfg: &Config) -> Result<String, ReorderError> {
    reorder_source_with_path(source, None, cfg)
}

/// Reorder. `source_path` is used only to name the file in the
/// stderr warning emitted when the end-of-pipeline parse self-check
/// trips and the rewrite is rolled back; nothing in the reorder
/// algorithm itself depends on the path.
pub fn reorder_source_with_path(
    source: &str,
    source_path: Option<&std::path::Path>,
    cfg: &Config,
) -> Result<String, ReorderError> {
    reorder_inner(source, cfg, source_path)
}

fn classify_trait_path(
    path: &syn::Path,
    std_imports: &HashSet<String>,
    crate_imports: &HashSet<String>,
    local_traits: &HashSet<String>,
) -> ImplKind {
    if let Some(first) = path.segments.first() {
        match first.ident.to_string().as_str() {
            "std" | "core" | "alloc" => return ImplKind::StdTrait,
            "crate" | "self" | "super" => return ImplKind::CrateTrait,
            _ => {}
        }
    }
    if path.segments.len() == 1 {
        let name = path.segments[0].ident.to_string();
        if std_imports.contains(&name) || PRELUDE_TRAITS.contains(&name.as_str()) {
            return ImplKind::StdTrait;
        }
        // Trait defined at the top of this same file (or imported from
        // crate-local scope under that name) → crate-trait.
        if crate_imports.contains(&name) || local_traits.contains(&name) {
            return ImplKind::CrateTrait;
        }
    }
    ImplKind::ExternalTrait
}

/// Walk a `use` tree, tagging each leaf by its origin (`std`/`core`/
/// `alloc` → Std, `crate`/`self`/`super` → Crate). Renames record the
/// new local name. Globs are ignored.
fn collect_use_imports(
    tree: &syn::UseTree,
    origin: ImportOrigin,
    std_out: &mut HashSet<String>,
    crate_out: &mut HashSet<String>,
) {
    use syn::UseTree::*;
    let push =
        |o: ImportOrigin, name: String, std: &mut HashSet<String>, c: &mut HashSet<String>| match o
        {
            ImportOrigin::Std => {
                std.insert(name);
            }
            ImportOrigin::Crate => {
                c.insert(name);
            }
            ImportOrigin::Unknown => {}
        };
    match tree {
        Path(p) => {
            let next = match origin {
                ImportOrigin::Unknown => match p.ident.to_string().as_str() {
                    "std" | "core" | "alloc" => ImportOrigin::Std,
                    "crate" | "self" | "super" => ImportOrigin::Crate,
                    _ => ImportOrigin::Unknown,
                },
                _ => origin,
            };
            collect_use_imports(&p.tree, next, std_out, crate_out);
        }
        Name(n) => push(origin, n.ident.to_string(), std_out, crate_out),
        Rename(r) => push(origin, r.rename.to_string(), std_out, crate_out),
        Glob(_) => {}
        Group(g) => {
            for inner in &g.items {
                collect_use_imports(inner, origin, std_out, crate_out);
            }
        }
    }
}

/// Recursively reorder bodies of every eligible inline `mod foo { ... }`
/// at the current scope, returning a rewritten source string. Each
/// recursive `reorder_inner` call handles its own nested inline mods,
/// so we only walk one level here.
/// Visit each top-level inline mod body and recursively reorder it.
/// Takes the already-parsed `File` to avoid a redundant parse —
/// `reorder_inner` will need its own `parsed` next anyway, and when
/// no inline mod actually changes (the common case) we can hand that
/// same AST back so the caller skips re-parsing entirely.
///
/// Returns `Ok(None)` when nothing changed; `Ok(Some(new_source))`
/// when at least one inline mod body was rewritten.
fn recurse_inline_mods(
    source: &str,
    parsed: &File,
    cfg: &Config,
) -> Result<Option<String>, ReorderError> {
    let mut replacements: Vec<(usize, usize, String)> = Vec::new();
    for item in &parsed.items {
        let Item::Mod(m) = item else { continue };
        let Some((brace, items)) = &m.content else {
            continue;
        };
        if should_skip_inline_recursion(m, items) {
            continue;
        }
        // Body bytes lie strictly between `{` and `}`. byte_range() of
        // the open brace ends at the byte just past `{`; byte_range() of
        // the close brace starts at the byte of `}`.
        let body_start = brace.span.open().byte_range().end;
        let body_end = brace.span.close().byte_range().start;
        if body_start > body_end || body_end > source.len() {
            continue; // synthesised span we can't trust — leave alone
        }
        let body = &source[body_start..body_end];
        let new_body = reorder_inner(body, cfg, None)?;
        if new_body != body {
            replacements.push((body_start, body_end, new_body));
        }
    }
    if replacements.is_empty() {
        return Ok(None);
    }
    // Apply in reverse byte order so earlier offsets stay valid.
    replacements.sort_by_key(|b| std::cmp::Reverse(b.0));
    let mut out = source.to_string();
    for (start, end, body) in replacements {
        out.replace_range(start..end, &body);
    }
    Ok(Some(out))
}
#[allow(clippy::too_many_arguments)]
fn compute_sort_key(
    item: &Item,
    idx: usize,
    segment: u32,
    category: Category,
    import_group: Option<ImportGroup>,
    mod_is_pub: Option<bool>,
    cfg: &Config,
    scope: &ScopeIndex<'_>,
) -> SortKey {
    // Order within a single category. The third visual group internally
    // sorts as crate (2) -> super (3) -> self (4) -> local-mod (5).
    let import_group_byte = match import_group {
        Some(ImportGroup::Std) => 0,
        Some(ImportGroup::External) => 1,
        Some(ImportGroup::Crate) => 2,
        Some(ImportGroup::Super) => 3,
        Some(ImportGroup::Self_) => 4,
        Some(ImportGroup::LocalMod) => 5,
        None => 0,
    };

    // `#[cfg(test)] mod tests` always last (unless disabled).
    let primary = if cfg.no_tests_last && category == Category::TestMod {
        Category::Mod.weight(cfg)
    } else {
        category.weight(cfg)
    };

    if !cfg.no_impl_grouping {
        if let Item::Impl(im) = item {
            // Try to anchor on the Self type (e.g. `impl X for Foo` -> Foo).
            let anchor = type_last_segment(&im.self_ty)
                .and_then(|n| scope.name_index.get(&n).copied())
                // Otherwise fall back to the trait name (e.g. `impl Greet for u32`
                // when only `Greet` is defined locally).
                .or_else(|| {
                    im.trait_.as_ref().and_then(|(_, path, _)| {
                        path.segments
                            .last()
                            .and_then(|s| scope.name_index.get(&s.ident.to_string()).copied())
                    })
                });
            let kind = match &im.trait_ {
                None => ImplKind::Inherent,
                Some((_, p, _)) => classify_trait_path(
                    p,
                    scope.std_imports,
                    scope.crate_imports,
                    scope.local_traits,
                ),
            };
            let trait_path_len = if !cfg.no_short_trait_first {
                im.trait_
                    .as_ref()
                    .map(|(_, p, _)| p.segments.len().min(u8::MAX as usize) as u8)
                    .unwrap_or(0)
            } else {
                0
            };

            if let Some((anchor_idx, anchor_cat)) = anchor {
                // Inherit the target type's group key so impls follow
                // their type to its new sorted position.
                let group_key = scope
                    .group_keys
                    .get(&anchor_idx)
                    .copied()
                    .unwrap_or_else(|| GroupSortKey::source_order(anchor_idx));
                return SortKey {
                    anchor: (group_key, anchor_idx),
                    segment,
                    primary: anchor_cat.weight(cfg),
                    follower: 1,
                    impl_kind: kind as u8,
                    import_group: 0,
                    trait_path_len,
                    original_index: idx,
                };
            }
            // Orphan impl: still classify by trait kind so all-std comes before
            // all-external when several orphan impls share the Impl bucket.
            return SortKey {
                anchor: (GroupSortKey::source_order(0), 0),
                segment,
                primary,
                follower: 0,
                impl_kind: kind as u8,
                import_group: 0,
                trait_path_len,
                original_index: idx,
            };
        }
    }

    // For top-level group-eligible categories
    // (struct/union/enum/trait/fn/async-fn), use the precomputed
    // group key so same-category
    // siblings sort by prefix-group + length + source order. Other
    // items get a neutral source-order key so they all tie on anchor and fall
    // through to `subgroup` / `original_index` — preserving e.g. the
    // pub-mod-first secondary-sort path for `mod` items.
    let anchor = if is_top_level_groupable(category) {
        let key = scope
            .group_keys
            .get(&idx)
            .copied()
            .unwrap_or_else(|| GroupSortKey::source_order(idx));
        (key, idx)
    } else {
        (GroupSortKey::source_order(0), 0)
    };

    // `import_group` field on SortKey is reused as a generic "secondary sort
    // bucket within a category". For Use it is the std/ext/crate-local index;
    // for Mod in the default pub-mod-first mode it is 0=pub, 1=private;
    // otherwise 0.
    let subgroup = if !cfg.no_import_groups && import_group.is_some() {
        import_group_byte
    } else if !cfg.no_pub_mod_first && category == Category::Mod {
        match mod_is_pub {
            Some(true) => 0,
            Some(false) | None => 1,
        }
    } else {
        0
    };

    SortKey {
        anchor,
        segment,
        primary,
        follower: 0,
        impl_kind: 0,
        import_group: subgroup,
        trait_path_len: 0,
        original_index: idx,
    }
}

fn compute_header_end_line(parsed: &File) -> usize {
    use syn::spanned::Spanned;
    let mut max_line = 0usize;
    if parsed.shebang.is_some() {
        max_line = max_line.max(1);
    }
    for attr in &parsed.attrs {
        max_line = max_line.max(attr.span().end().line);
    }
    max_line
}

/// Categories that participate in top-level same-category prefix-grouping.
fn is_top_level_groupable(cat: Category) -> bool {
    matches!(
        cat,
        Category::Struct | Category::Enum | Category::Trait | Category::Fn | Category::AsyncFn
    )
}

/// Decide whether to recurse into an inline `mod foo { ... }` body.
///
/// Skipped because reordering would change semantics or destroy
/// deliberately-curated ordering:
///   * `#[cfg(test)]` mods or those literally named `tests`/`test` —
///     handled separately by the file-level `tests_last` rule.
///   * `#[macro_use]` mods — `macro_rules!` inside leak to the parent
///     scope, so moving them within the body changes visibility order.
///   * Pure-`use` mods (every item is `Item::Use`) — covers `prelude`,
///     `__private`, sealed-trait re-export shims, etc., where the
///     listing order is part of the module's public contract.
fn should_skip_inline_recursion(m: &syn::ItemMod, items: &[Item]) -> bool {
    let name = m.ident.to_string();
    if name == "tests" || name == "test" || has_cfg_test(&m.attrs) {
        return true;
    }
    if m.attrs.iter().any(|a| a.path().is_ident("macro_use")) {
        return true;
    }
    if !items.is_empty() && items.iter().all(|i| matches!(i, Item::Use(_))) {
        return true;
    }
    false
}
