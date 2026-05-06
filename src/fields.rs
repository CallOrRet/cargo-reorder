//! Group-and-sort named fields inside `struct` / `union` / `enum`
//! (and inside struct-like enum variants).
//!
//! Rules:
//! - **Group** by snake_case prefix — the substring of the field name
//!   before the first `_`. A field whose name has no `_` is its own
//!   one-element group keyed on the whole name.
//! - Within a group, sort by **name length ascending** (shorter first).
//!   Ties (same length) preserve source order via a stable sort.
//! - Groups are emitted in ascending order of **the group's mean
//!   name length** (sum of names / member count). Ties between
//!   groups preserve source order — whichever group's first member
//!   appeared earliest in the source goes first.
//! - A blank line is inserted between consecutive groups. Within a
//!   group, fields stay packed.
//!
//! Skip rules — when any of these apply, the item is left exactly as
//! the user wrote it. Reordering would change ABI, layout, or
//! derived-trait semantics:
//! - The container carries any `#[repr(...)]` (`C`, `packed`,
//!   `transparent`, `align(N)`, integer reprs).
//! - The container carries `#[derive(PartialOrd)]` or
//!   `#[derive(Ord)]` — derived comparisons depend on declaration
//!   order.
//! - On enums, **any** variant has an explicit discriminant
//!   (`A = 1`); reordering would silently change implicit values for
//!   the others.
//! - The body has fewer than two named fields (nothing to do).
//! - Tuple variants / unit variants / tuple structs — no field
//!   names to group on.

use syn::spanned::Spanned;
use syn::{
    Attribute, Fields, FieldsNamed, GenericParam, ImplItem, Item, ItemEnum, ItemImpl, ItemStruct,
    ItemTrait, ItemUnion, TraitBoundModifier, TraitItem, TypeParamBound, WherePredicate,
};

/// Top-level entry point: given a parsed item and its raw source text,
/// return a rewritten version with fields/variants reordered, or
/// `None` if no rewrite was performed.
pub(crate) fn reorder_in_item(item: &Item, body_text: &str, start_line: usize) -> Option<String> {
    match item {
        Item::Struct(s) => rewrite_struct(s, body_text, start_line),
        Item::Union(u) => rewrite_union(u, body_text, start_line),
        Item::Enum(e) => rewrite_enum(e, body_text, start_line),
        Item::Impl(i) => rewrite_impl(i, body_text, start_line),
        Item::Trait(t) => rewrite_trait(t, body_text, start_line),
        _ => None,
    }
}

fn rewrite_struct(s: &ItemStruct, body_text: &str, start_line: usize) -> Option<String> {
    if container_skips(&s.attrs) {
        return None;
    }
    let Fields::Named(named) = &s.fields else {
        return None;
    };
    // DST layout: at most one unsized field, and it must be last. The
    // user's source already has the unsized field last (it wouldn't
    // compile otherwise), so pin the last named field in place and let
    // the rest reorder normally.
    let pin_last = has_unsized_generic(&s.generics);
    rewrite_named_fields_inplace(body_text, start_line, named, pin_last)
}

fn rewrite_union(u: &ItemUnion, body_text: &str, start_line: usize) -> Option<String> {
    if container_skips(&u.attrs) {
        return None;
    }
    let pin_last = has_unsized_generic(&u.generics);
    rewrite_named_fields_inplace(body_text, start_line, &u.fields, pin_last)
}

fn rewrite_impl(item: &ItemImpl, body_text: &str, start_line: usize) -> Option<String> {
    // Members get a `bucket` matching the top-level category order:
    // const (0) → type (1) → fn (2) → async fn (3). Any macro /
    // verbatim member is a hard barrier — its presence means we leave
    // the body alone (we don't know its effects, can't safely cross).
    if item
        .items
        .iter()
        .any(|i| matches!(i, ImplItem::Macro(_) | ImplItem::Verbatim(_)))
    {
        return None;
    }
    sort_top_level(
        body_text,
        start_line,
        item.items.iter().filter_map(|i| match i {
            ImplItem::Const(c) => Some(member_entry(&c.attrs, &c.ident, c.span(), 0)),
            ImplItem::Type(t) => Some(member_entry(&t.attrs, &t.ident, t.span(), 1)),
            ImplItem::Fn(f) => Some(member_entry(
                &f.attrs,
                &f.sig.ident,
                f.span(),
                if f.sig.asyncness.is_some() { 3 } else { 2 },
            )),
            _ => None,
        }),
    )
}

fn rewrite_trait(item: &ItemTrait, body_text: &str, start_line: usize) -> Option<String> {
    // Same buckets as `rewrite_impl`. A trait body can carry
    // `const` / `type` declarations and method signatures (with or
    // without default bodies) — all sort by category, then by
    // prefix-group + length within their bucket.
    if item
        .items
        .iter()
        .any(|i| matches!(i, TraitItem::Macro(_) | TraitItem::Verbatim(_)))
    {
        return None;
    }
    sort_top_level(
        body_text,
        start_line,
        item.items.iter().filter_map(|i| match i {
            TraitItem::Const(c) => Some(member_entry(&c.attrs, &c.ident, c.span(), 0)),
            TraitItem::Type(t) => Some(member_entry(&t.attrs, &t.ident, t.span(), 1)),
            TraitItem::Fn(f) => Some(member_entry(
                &f.attrs,
                &f.sig.ident,
                f.span(),
                if f.sig.asyncness.is_some() { 3 } else { 2 },
            )),
            _ => None,
        }),
    )
}

fn member_entry(
    attrs: &[Attribute],
    ident: &syn::Ident,
    span: proc_macro2::Span,
    bucket: u8,
) -> SortableLines {
    let earliest_attr = attrs.first().map(|a| a.span());
    let first_line = earliest_attr
        .map(|s| s.start().line)
        .unwrap_or_else(|| span.start().line);
    SortableLines {
        first_line,
        last_line: span.end().line,
        name: ident.to_string(),
        bucket,
    }
}

fn rewrite_enum(e: &ItemEnum, body_text: &str, start_line: usize) -> Option<String> {
    if container_skips(&e.attrs) {
        return None;
    }
    if e.variants.iter().any(|v| v.discriminant.is_some()) {
        return None;
    }
    let mut out = body_text.to_string();

    // Step 1: rewrite each struct-like variant's named-fields block
    // from the back so earlier offsets stay valid.
    let mut variant_rewrites: Vec<(usize, usize, String)> = Vec::new();
    for v in &e.variants {
        if let Fields::Named(named) = &v.fields {
            // Compute the line range of this named-fields block in
            // body_text. We need to slice it out, rewrite, splice back.
            let line_range = field_block_line_range(&out, start_line, named);
            if let Some((line_lo, line_hi)) = line_range {
                let block_text = lines_slice(&out, start_line, line_lo, line_hi);
                if let Some(rewritten) =
                    rewrite_named_fields_inplace(&block_text, line_lo, named, false)
                {
                    let (lo_byte, hi_byte) =
                        byte_range_for_line_range(&out, start_line, line_lo, line_hi);
                    variant_rewrites.push((lo_byte, hi_byte, rewritten));
                }
            }
        }
    }
    variant_rewrites.sort_by_key(|(lo, _, _)| std::cmp::Reverse(*lo));
    for (lo, hi, replacement) in variant_rewrites {
        out.replace_range(lo..hi, &replacement);
    }

    // Step 2: sort the variants themselves. Re-parse `out` to get
    // fresh spans matching the post-rewrite text. Note: `syn::parse_str`
    // restarts span line numbering at 1 within the parsed string, so
    // the `text_start_line` we pass to `sort_top_level` must be `1`,
    // *not* the original-source `start_line` we got from the caller —
    // the entries' `first_line` / `last_line` are 1-indexed within
    // `out`, not within the surrounding source file.
    let parsed: ItemEnum = syn::parse_str(&out).ok()?;
    // serde-derive forces `#[serde(other)]` to be on the last variant;
    // pin that variant in place and reorder only its predecessors.
    sort_top_level(
        &out,
        1,
        parsed.variants.iter().map(|v| {
            let name = v.ident.to_string();
            let earliest_attr = v.attrs.first().map(|a| a.span());
            let span = v.span();
            let first_line = earliest_attr
                .map(|s| s.start().line)
                .unwrap_or_else(|| span.start().line);
            let last_line = span.end().line;
            SortableLines {
                first_line,
                last_line,
                name,
                bucket: if variant_pinned_last(v) { PIN_LAST } else { 0 },
            }
        }),
    )
}

/// Take a named-fields block (`{ a: u8, b: u16 }`) and rewrite the
/// containing item's text with the fields reordered. When `pin_last`
/// is true, the final field stays in place and only the prefix is
/// reordered — used for DST layouts where the trailing field must
/// remain last (`?Sized`, `[T]`, `str`, `dyn Trait`).
fn rewrite_named_fields_inplace(
    text: &str,
    text_start_line: usize,
    named: &FieldsNamed,
    pin_last: bool,
) -> Option<String> {
    if named.named.len() < 2 {
        return None;
    }
    let last_idx = named.named.len() - 1;
    sort_top_level(
        text,
        text_start_line,
        named.named.iter().enumerate().map(|(i, f)| {
            let name = f.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
            let earliest_attr = f.attrs.first().map(|a| a.span());
            let span = f.span();
            let first_line = earliest_attr
                .map(|s| s.start().line)
                .unwrap_or_else(|| span.start().line);
            let last_line = span.end().line;
            SortableLines {
                first_line,
                last_line,
                name,
                bucket: if pin_last && i == last_idx { PIN_LAST } else { 0 },
            }
        }),
    )
}

/// Generic line-based reordering. `text` is a chunk of source whose
/// first line is `text_start_line`. `entries` are the items to sort
/// (each carries its source line range and a name to group/sort on).
///
/// Returns the reordered text. Lines outside the entries' ranges
/// (header, closing brace, comments before/after the field block) are
/// preserved.
fn sort_top_level<I>(text: &str, text_start_line: usize, entries: I) -> Option<String>
where
    I: IntoIterator<Item = SortableLines>,
{
    let entries: Vec<SortableLines> = entries.into_iter().collect();
    if entries.len() < 2 {
        return None;
    }
    // Bail if any two entries share a source line — line-based slicing
    // can't disentangle them. This protects single-line layouts like
    // `enum E { A { x: u8, y: u8 }, B }` and `struct S { a: u8, b: u8 }`
    // (all-on-one-line), which we deliberately leave untouched.
    let mut prev_last = 0usize;
    for e in &entries {
        if e.first_line <= prev_last {
            return None;
        }
        prev_last = e.last_line;
    }

    // Convert text into a Vec of full-line strings (each ending with \n
    // except possibly the last). This makes range-based splicing trivial.
    let lines: Vec<String> = split_lines(text);
    let total_lines = lines.len();

    // Translate each entry's source line numbers to indices into
    // `lines`. A line that's `text_start_line + i` maps to `lines[i]`.
    // We *expand* each entry's line range to also cover comment / blank
    // lines that immediately precede it (so `///` doc comments travel
    // with the field). The expansion stops when we hit either:
    //   - the previous entry's last line, OR
    //   - the line that contains the opening `{` of the block, OR
    //   - a blank line that is itself preceded by a non-comment line
    //     (i.e. the blank line belongs to the structural separator,
    //     not to this field's leading trivia).
    let to_idx = |line: usize| -> Option<usize> {
        line.checked_sub(text_start_line)
            .filter(|&i| i < total_lines)
    };

    let mut ranges: Vec<(usize, usize, String, u8)> = Vec::with_capacity(entries.len());
    let mut prev_end: Option<usize> = None;
    for e in &entries {
        let first_idx = to_idx(e.first_line)?;
        let last_idx = to_idx(e.last_line)?;
        // Expand backwards over preceding `///`, `//`, `#[...]`, blank
        // lines that look like part of this field's leading trivia.
        let mut start = first_idx;
        while start > prev_end.map(|p| p + 1).unwrap_or(0) {
            let prev = lines[start - 1].trim_start();
            if prev.starts_with("///")
                || prev.starts_with("//!")
                || prev.starts_with("#[")
                || prev.starts_with("//")
            {
                start -= 1;
            } else {
                break;
            }
        }
        ranges.push((start, last_idx, e.name.clone(), e.bucket));
        prev_end = Some(last_idx);
    }

    // Find the first and last line index that any entry covers — that
    // is, the field-block range. Lines outside this range are header
    // and footer (`{`/`}` etc).
    let first_field_line = ranges.first()?.0;
    let last_field_line = ranges.last()?.1;

    // Header: lines [0, first_field_line). Footer: lines
    // (last_field_line, end]. Body becomes the reordered concatenation
    // of each entry's slice plus an additional blank line between groups.
    let mut header: String = lines[..first_field_line].concat();
    let footer: String = lines[last_field_line + 1..].concat();

    // Group by (bucket, prefix). Each unique key becomes a group.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut group_key_order: Vec<(u8, String)> = Vec::new();
    for (idx, (_, _, name, bucket)) in ranges.iter().enumerate() {
        let key = (*bucket, prefix_of(name).to_string());
        if let Some(pos) = group_key_order.iter().position(|p| *p == key) {
            groups[pos].push(idx);
        } else {
            group_key_order.push(key);
            groups.push(vec![idx]);
        }
    }
    // Within each group: stable-sort by name length ascending.
    for g in &mut groups {
        g.sort_by_key(|&i| ranges[i].2.len());
    }
    // Between groups: bucket first (lower bucket sorts first —
    // implements "all sync `fn` before all `async fn`" inside impl /
    // trait bodies), then stable-sort by mean name length within the
    // same bucket. Ties preserve source order — whichever group's
    // first member appeared earliest wins. We avoid floats by
    // comparing `sum_a * count_b` vs `sum_b * count_a`.
    groups.sort_by(|a, b| {
        let bucket_a = ranges[a[0]].3;
        let bucket_b = ranges[b[0]].3;
        if bucket_a != bucket_b {
            return bucket_a.cmp(&bucket_b);
        }
        let sum_a: usize = a.iter().map(|&i| ranges[i].2.len()).sum();
        let sum_b: usize = b.iter().map(|&i| ranges[i].2.len()).sum();
        (sum_a * b.len()).cmp(&(sum_b * a.len()))
    });

    // Emit: each entry's slice (which already ends with \n), with a
    // blank line between groups.
    let mut body = String::new();
    for (gi, g) in groups.iter().enumerate() {
        if gi > 0 {
            body.push('\n');
        }
        for &i in g {
            let (lo, hi, _, _) = &ranges[i];
            for line in &lines[*lo..=*hi] {
                body.push_str(line);
            }
        }
    }
    // Strip any trailing blank lines from `body` so the footer's
    // leading whitespace stays clean.
    while body.ends_with("\n\n") {
        body.pop();
    }
    // Make sure body ends with exactly one newline.
    if !body.ends_with('\n') {
        body.push('\n');
    }
    // Make sure header ends with newline (it should, but be safe).
    if !header.is_empty() && !header.ends_with('\n') {
        header.push('\n');
    }
    Some(format!("{header}{body}{footer}"))
}

/// Sentinel bucket value that pins an entry to the very end of the
/// reordered output. Used for fields/variants that must keep their
/// trailing position for semantic reasons (DST layout for `?Sized`
/// fields, `#[serde(other)]` on enum variants).
const PIN_LAST: u8 = u8::MAX;

#[derive(Clone)]
struct SortableLines {
    first_line: usize,
    last_line: usize,
    name: String,
    /// Sub-bucket within the entry list. Items in lower buckets sort
    /// before items in higher buckets, regardless of their group's
    /// mean. Used by impl/trait body reordering to keep `async fn`
    /// after sync `fn`. Field-level callers usually pass `0`; the
    /// special value [`PIN_LAST`] pins the entry to the end.
    bucket: u8,
}

/// First "word" of an identifier — text up to the first `_` (snake_case)
/// OR up to the first lowercase→uppercase boundary (camelCase/PascalCase).
/// Ensures `foo_bar` → "foo", `FooBar` → "Foo", `fooBar` → "foo",
/// while leaving `Foo`, `BAR`, `XMLParser` as a single word.
pub(crate) fn prefix_of(name: &str) -> &str {
    let mut prev_was_lower = false;
    for (i, c) in name.char_indices() {
        if c == '_' {
            return &name[..i];
        }
        if c.is_uppercase() && prev_was_lower {
            return &name[..i];
        }
        prev_was_lower = c.is_lowercase();
    }
    name
}

/// Compute a per-item sort key `(group_mean_x_count, name_len)` for
/// each `(idx, name)` pair, where:
/// - All items sharing the same `prefix_of(name)` form one group.
/// - The group's first key component is its mean name length expressed
///   as `sum_of_lens` (so the *order* of groups by mean matches integer
///   ordering of `sum_of_lens / count`, which we cross-multiply to keep
///   integers exact at compare time).
///
/// To allow direct use of the returned `(u32, u32)` as a `SortKey`
/// component while still ordering by mean, we encode the mean as a
/// fixed-point integer: `sum * 1_000_000 / count` (clamped to u32
/// range). Comparing these integers gives the same order as comparing
/// the true means.
///
/// `name_len` is the second component, used as the in-group secondary
/// key: shorter names sort first within a group.
///
/// Items whose name is empty produce a `(u32::MAX, 0)` key so they
/// sort to the end of their bucket — but in practice all callers pass
/// non-empty names.
pub(crate) fn compute_group_keys<'a, I>(items: I) -> std::collections::HashMap<usize, (u32, u32)>
where
    I: IntoIterator<Item = (usize, &'a str)>,
{
    let items: Vec<(usize, &'a str)> = items.into_iter().collect();
    let mut groups: std::collections::HashMap<&'a str, (u64, u32)> =
        std::collections::HashMap::new();
    for (_, name) in &items {
        let entry = groups.entry(prefix_of(name)).or_insert((0u64, 0u32));
        entry.0 += name.len() as u64;
        entry.1 += 1;
    }
    let mut out = std::collections::HashMap::with_capacity(items.len());
    for (idx, name) in items {
        let pfx = prefix_of(name);
        let (sum, count) = groups[pfx];
        // Encode mean as sum * 1_000_000 / count for fixed-point ordering.
        let mean_proxy = if count == 0 {
            0
        } else {
            ((sum as u128 * 1_000_000) / count as u128).min(u32::MAX as u128) as u32
        };
        out.insert(idx, (mean_proxy, name.len() as u32));
    }
    out
}

/// Split `text` into lines, each retaining its trailing `\n` if any.
fn split_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            out.push(text[start..=i].to_string());
            start = i + 1;
        }
    }
    if start < text.len() {
        out.push(text[start..].to_string());
    }
    out
}

/// Find the line range (inclusive, 1-based source lines) that an
/// inline named-fields block occupies in `text`, given the parsed
/// `FieldsNamed` whose span coordinates refer to the original source.
fn field_block_line_range(
    text: &str,
    text_start_line: usize,
    named: &FieldsNamed,
) -> Option<(usize, usize)> {
    let span = named.span();
    let lo = span.start().line;
    let hi = span.end().line;
    let total = split_lines(text).len();
    let _lo_idx = lo.checked_sub(text_start_line)?;
    let hi_idx = hi.checked_sub(text_start_line)?;
    if hi_idx >= total {
        return None;
    }
    Some((lo, hi))
}

fn lines_slice(text: &str, text_start_line: usize, line_lo: usize, line_hi: usize) -> String {
    let lines = split_lines(text);
    let lo = line_lo.saturating_sub(text_start_line);
    let hi = line_hi.saturating_sub(text_start_line).min(lines.len() - 1);
    lines[lo..=hi].concat()
}

fn byte_range_for_line_range(
    text: &str,
    text_start_line: usize,
    line_lo: usize,
    line_hi: usize,
) -> (usize, usize) {
    let lines = split_lines(text);
    let lo_idx = line_lo.saturating_sub(text_start_line).min(lines.len());
    let hi_idx = line_hi
        .saturating_sub(text_start_line)
        .min(lines.len().saturating_sub(1));
    let mut byte = 0usize;
    for l in &lines[..lo_idx] {
        byte += l.len();
    }
    let mut end_byte = byte;
    for l in &lines[lo_idx..=hi_idx] {
        end_byte += l.len();
    }
    (byte, end_byte)
}

/// True if the container's generics carry a `?Sized` bound — either as a
/// direct param bound (`<T: ?Sized>`) or in a `where` clause
/// (`where T: ?Sized`). DST layout pins the unsized field as the last
/// field of the struct/union, so we must not reorder named fields when
/// the type can be unsized.
fn has_unsized_generic(generics: &syn::Generics) -> bool {
    let bounds_have_maybe_sized = |bounds: &syn::punctuated::Punctuated<TypeParamBound, _>| {
        bounds.iter().any(|b| match b {
            TypeParamBound::Trait(t) => {
                matches!(t.modifier, TraitBoundModifier::Maybe(_)) && t.path.is_ident("Sized")
            }
            _ => false,
        })
    };
    let direct = generics.params.iter().any(|p| match p {
        GenericParam::Type(t) => bounds_have_maybe_sized(&t.bounds),
        _ => false,
    });
    if direct {
        return true;
    }
    generics
        .where_clause
        .as_ref()
        .is_some_and(|w| {
            w.predicates.iter().any(|p| match p {
                WherePredicate::Type(pt) => bounds_have_maybe_sized(&pt.bounds),
                _ => false,
            })
        })
}

/// True if the variant carries a serde-derive attribute that requires it
/// to stay in a fixed position. Currently: `#[serde(other)]` must be on
/// the last variant of an externally-tagged enum.
fn variant_pinned_last(v: &syn::Variant) -> bool {
    v.attrs.iter().any(|a| {
        if !a.path().is_ident("serde") {
            return false;
        }
        let mut found = false;
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("other") {
                found = true;
            }
            Ok(())
        });
        found
    })
}

fn container_skips(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        if a.path().is_ident("repr") {
            return true;
        }
        if a.path().is_ident("derive") {
            let mut found = false;
            let _ = a.parse_nested_meta(|meta| {
                if meta.path.is_ident("PartialOrd") || meta.path.is_ident("Ord") {
                    found = true;
                }
                Ok(())
            });
            return found;
        }
        false
    })
}
