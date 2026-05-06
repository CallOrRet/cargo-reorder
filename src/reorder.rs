//! Reordering pipeline: parse with `syn`, slice the source into per-item
//! `Block`s (leading comments + body + trailing gap), assign sort keys,
//! sort, post-process macro placement, reassemble.

use std::collections::{HashMap, HashSet};
use std::fmt;

use syn::{File, Item};

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

#[derive(Debug)]
pub enum ReorderError {
    Parse(syn::Error),
}

impl std::error::Error for ReorderError {}

impl From<syn::Error> for ReorderError {
    fn from(e: syn::Error) -> Self {
        ReorderError::Parse(e)
    }
}

impl fmt::Display for ReorderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReorderError::Parse(e) => write!(f, "failed to parse Rust source: {e}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

    /// Sort weight, depending on user config (mod-before-use vs use-first).
    fn weight(self, cfg: &Config) -> u32 {
        let use_first = !cfg.mod_before_use;
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
                if cfg.struct_before_trait {
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
}

/// How an `impl` block is classified relative to a target type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ImplKind {
    Inherent = 0,
    StdTrait = 1,
    CrateTrait = 2,
    ExternalTrait = 3,
}

/// Use-tree origin tag — used by both std-import and crate-import scans.
#[derive(Copy, Clone, PartialEq, Eq)]
enum ImportOrigin {
    Unknown,
    Std,
    Crate,
}

/// User-tunable behaviour. See README for the rationale on each flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// `mod foo;` before `use`. Minority convention; see README.
    pub mod_before_use: bool,
    /// Split imports into std / external / crate-local groups.
    pub group_imports: bool,
    /// `impl` blocks follow their target type, ordered
    /// inherent → std trait → external trait.
    pub group_impls_with_type: bool,
    /// Force `#[cfg(test)] mod ...` to the end of the file.
    pub tests_last: bool,
    /// Sort `pub mod` before `mod` with a blank line between.
    pub pub_mod_first: bool,
    /// Sort `enum` / `struct` before `trait`. Default off (trait-first
    /// matches the slight majority in real-world Rust files — see README).
    pub struct_before_trait: bool,
    /// Recurse into inline `mod foo { ... }` blocks and reorder their bodies
    /// with the same rules. On by default — see README for the skip list
    /// (test mods, `#[macro_use]` mods, pure-`use` mods).
    pub reorder_inline_mods: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mod_before_use: false,
            group_imports: true,
            group_impls_with_type: true,
            tests_last: true,
            pub_mod_first: false,
            struct_before_trait: false,
            reorder_inline_mods: true,
        }
    }
}

pub(crate) struct Block {
    pub(crate) category: Category,
    pub(crate) import_group: Option<ImportGroup>,
    /// Visibility for `mod` items, used by the blank-line logic when
    /// `pub_mod_first` is enabled. `Some(true)` = pub mod, `Some(false)` =
    /// private mod, `None` = not a mod.
    pub(crate) mod_is_pub: Option<bool>,
    pub(crate) leading: String,
    pub(crate) body: String,
    pub(crate) trailing: String,
    pub(crate) sort_key: SortKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SortKey {
    /// Items in different segments never cross each other. Bare top-level
    /// macro invocations get a private odd segment (pinned in place);
    /// everything else lives in the surrounding even segments.
    segment: u32,
    /// Primary category weight.
    primary: u32,
    /// Anchor index — for impls, the original index of their target type;
    /// for normal items, their own original index.
    anchor: usize,
    /// 0 = the type definition itself, 1 = a follower impl.
    follower: u8,
    /// Within a type's followers: inherent (0) → std trait (1) → external (2).
    impl_kind: u8,
    /// Import group (only meaningful for `use` items).
    import_group: u8,
    /// Tie-breaker: original source index (preserves stable order).
    pub(crate) original_index: usize,
}

/// Per-item inputs to [`compute_sort_key`]. Bundling these keeps the
/// function signature small and the call site readable.
struct SortItem<'a> {
    item: &'a Item,
    idx: usize,
    segment: u32,
    category: Category,
    import_group: Option<ImportGroup>,
    mod_is_pub: Option<bool>,
}

/// Per-scope reference data shared across every call to
/// [`compute_sort_key`]: name → (idx, category) lookup for impl
/// anchoring, plus the three trait-classification name sets.
struct ScopeIndex<'a> {
    name_index: &'a HashMap<String, (usize, Category)>,
    std_imports: &'a HashSet<String>,
    crate_imports: &'a HashSet<String>,
    local_traits: &'a HashSet<String>,
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

/// Reorder with the default [`Config`].
pub fn reorder_source(source: &str) -> Result<String, ReorderError> {
    reorder_source_with_path(source, None, &Config::default())
}

pub fn reorder_source_with(source: &str, cfg: &Config) -> Result<String, ReorderError> {
    reorder_source_with_path(source, None, cfg)
}

/// Reorder. `source_path` is accepted for API compatibility (and so a
/// caller can identify the file in any future path-aware behaviour),
/// but the current implementation does not consult it — every macro
/// item is a barrier regardless of where the file lives.
pub fn reorder_source_with_path(
    source: &str,
    _source_path: Option<&std::path::Path>,
    cfg: &Config,
) -> Result<String, ReorderError> {
    reorder_inner(source, cfg)
}

/// Internal entry point. Recurses into itself for cargo-script
/// frontmatter stripping and (when `cfg.reorder_inline_mods` is on)
/// for inline `mod foo { ... }` body recursion.
fn reorder_inner(source: &str, cfg: &Config) -> Result<String, ReorderError> {
    // Cargo-script: strip frontmatter, reorder body, stitch back.
    if let Some((prefix, body)) = crate::frontmatter::split(source) {
        let reordered = reorder_inner(&body, cfg)?;
        return Ok(format!("{prefix}{reordered}"));
    }

    // Optional pre-pass: recursively reorder eligible inline mod bodies
    // before tackling the file-level item order. Each recursive call
    // handles its own further-nested inline mods, so we only need to
    // process top-level inline mods here. We then re-parse the rewritten
    // source so the rest of the pipeline sees the canonicalised bodies.
    let owned: String;
    let working: &str = if cfg.reorder_inline_mods {
        owned = recurse_inline_mods(source, cfg)?;
        &owned
    } else {
        source
    };

    let parsed: File = syn::parse_file(working)?;
    if parsed.items.is_empty() {
        return Ok(working.to_string());
    }
    let source = working;

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
    for (idx, item) in parsed.items.iter().enumerate() {
        let cat = Category::classify(item);
        match item {
            Item::Struct(s) => {
                name_index.insert(s.ident.to_string(), (idx, cat));
            }
            Item::Enum(e) => {
                name_index.insert(e.ident.to_string(), (idx, cat));
            }
            Item::Union(u) => {
                name_index.insert(u.ident.to_string(), (idx, cat));
            }
            Item::Trait(t) => {
                name_index.insert(t.ident.to_string(), (idx, cat));
                local_traits.insert(t.ident.to_string());
            }
            Item::TraitAlias(t) => {
                name_index.insert(t.ident.to_string(), (idx, cat));
                local_traits.insert(t.ident.to_string());
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
        std_imports: &std_imports,
        crate_imports: &crate_imports,
        local_traits: &local_traits,
    };
    let mut blocks: Vec<Block> = Vec::with_capacity(parsed.items.len());
    for (idx, (item, &(start, end))) in parsed.items.iter().zip(ranges.iter()).enumerate() {
        let leading = if idx == 0 {
            first_leading.clone()
        } else {
            String::new()
        };
        let body = take_lines(&lines, start, end);
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
        let sort_input = SortItem {
            item,
            idx,
            // segments[idx] is already widened by FENCE_STRIDE and
            // bumped past any preceding fences.
            segment: segments[idx],
            category,
            import_group,
            mod_is_pub,
        };
        let sort_key = compute_sort_key(&sort_input, cfg, &scope);
        blocks.push(Block {
            category,
            mod_is_pub,
            import_group,
            leading,
            body,
            trailing: String::new(),
            sort_key,
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
    // `original_index = n_items + fence_idx` keeps fences out of the
    // macro pipeline's `[0, n_items)` reverse-index but stays close to
    // n_items so `yank_macro_defs`'s `pos` Vec doesn't balloon.
    let n_items = parsed.items.len();
    let mut fence_idx = 0usize;
    for (i, fence) in fence_after.iter().enumerate() {
        if let Some((comment_text, _)) = fence {
            let fence_segment = segments[i].saturating_add(FENCE_STRIDE / 2);
            blocks.push(Block {
                category: Category::Fence,
                mod_is_pub: None,
                import_group: None,
                leading: comment_text.clone(),
                body: String::new(),
                trailing: String::new(),
                sort_key: SortKey {
                    segment: fence_segment,
                    primary: 0,
                    anchor: 0,
                    follower: 0,
                    impl_kind: 0,
                    import_group: 0,
                    original_index: n_items + fence_idx,
                },
            });
            fence_idx += 1;
        }
    }

    blocks.sort_by_key(|b| b.sort_key);

    let header = take_lines(&lines, 1, header_end_line);
    Ok(crate::emit::assemble(
        &header,
        &header_extra,
        &blocks,
        &footer,
        cfg,
    ))
}

fn compute_sort_key(it: &SortItem<'_>, cfg: &Config, scope: &ScopeIndex<'_>) -> SortKey {
    // Order within a single category. The third visual group internally
    // sorts as crate (2) -> super (3) -> self (4) -> local-mod (5).
    let import_group_byte = match it.import_group {
        Some(ImportGroup::Std) => 0,
        Some(ImportGroup::External) => 1,
        Some(ImportGroup::Crate) => 2,
        Some(ImportGroup::Super) => 3,
        Some(ImportGroup::Self_) => 4,
        Some(ImportGroup::LocalMod) => 5,
        None => 0,
    };

    // `#[cfg(test)] mod tests` always last (unless disabled).
    let primary = if !cfg.tests_last && it.category == Category::TestMod {
        Category::Mod.weight(cfg)
    } else {
        it.category.weight(cfg)
    };

    if cfg.group_impls_with_type {
        if let Item::Impl(im) = it.item {
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

            if let Some((anchor_idx, anchor_cat)) = anchor {
                return SortKey {
                    segment: it.segment,
                    primary: anchor_cat.weight(cfg),
                    anchor: anchor_idx,
                    follower: 1,
                    impl_kind: kind as u8,
                    import_group: 0,
                    original_index: it.idx,
                };
            }
            // Orphan impl: still classify by trait kind so all-std comes before
            // all-external when several orphan impls share the Impl bucket.
            return SortKey {
                segment: it.segment,
                primary,
                anchor: it.idx,
                follower: 0,
                impl_kind: kind as u8,
                import_group: 0,
                original_index: it.idx,
            };
        }
    }

    // Only type definitions need a non-zero `anchor`, so that `impl` blocks
    // can anchor to them. Other items use `anchor = 0` so within their
    // category they fall through to `subgroup` / `original_index` for
    // ordering — otherwise a `use` with smaller original_index would sort
    // ahead of one in an earlier import group.
    let anchor = match it.category {
        Category::Struct | Category::Enum | Category::Trait => it.idx,
        _ => 0,
    };

    // `import_group` field on SortKey is reused as a generic "secondary sort
    // bucket within a category". For Use it is the std/ext/crate-local index;
    // for Mod with `pub_mod_first` it is 0=pub, 1=private; otherwise 0.
    let subgroup = if cfg.group_imports && it.import_group.is_some() {
        import_group_byte
    } else if cfg.pub_mod_first && it.category == Category::Mod {
        match it.mod_is_pub {
            Some(true) => 0,
            Some(false) | None => 1,
        }
    } else {
        0
    };

    SortKey {
        segment: it.segment,
        primary,
        anchor,
        follower: 0,
        impl_kind: 0,
        import_group: subgroup,
        original_index: it.idx,
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

/// Recursively reorder bodies of every eligible inline `mod foo { ... }`
/// at the current scope, returning a rewritten source string. Each
/// recursive `reorder_inner` call handles its own nested inline mods,
/// so we only walk one level here.
fn recurse_inline_mods(source: &str, cfg: &Config) -> Result<String, ReorderError> {
    let parsed: File = syn::parse_file(source)?;
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
        let new_body = reorder_inner(body, cfg)?;
        if new_body != body {
            replacements.push((body_start, body_end, new_body));
        }
    }
    // Apply in reverse byte order so earlier offsets stay valid.
    replacements.sort_by_key(|b| std::cmp::Reverse(b.0));
    let mut out = source.to_string();
    for (start, end, body) in replacements {
        out.replace_range(start..end, &body);
    }
    Ok(out)
}
