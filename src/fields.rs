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
//! - Existing blank lines before a field move with that field. Field
//!   sorting does not add blank separators between groups.
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
use syn::visit::{self, Visit};
use syn::{
    Attribute, ExprStruct, Fields, FieldsNamed, FnArg, GenericParam, ImplItem, Item, ItemEnum,
    ItemImpl, ItemStruct, ItemTrait, ItemUnion, Member, Pat, Signature, TraitBoundModifier,
    TraitItem, TypeParamBound, WherePredicate,
};

/// Sentinel bucket value that pins an entry to the very end of the
/// reordered output. Used for fields/variants that must keep their
/// trailing position for semantic reasons (DST layout for `?Sized`
/// fields, `#[serde(other)]` on enum variants).
const PIN_LAST: u8 = u8::MAX;

#[derive(Clone)]
struct SortableLines {
    name: String,

    /// Sub-bucket within the entry list. Items in lower buckets sort
    /// before items in higher buckets, regardless of their group's
    /// mean. Used by impl/trait body reordering to keep `async fn`
    /// after sync `fn`. Field-level callers usually pass `0`; the
    /// special value [`PIN_LAST`] pins the entry to the end.
    bucket: u8,

    last_line: usize,

    first_line: usize,
}

#[derive(Clone)]
struct SortableSpan {
    name: String,

    lo: usize,

    hi: usize,
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

fn lines_slice(text: &str, text_start_line: usize, line_lo: usize, line_hi: usize) -> String {
    let lines = split_lines(text);
    let lo = line_lo.saturating_sub(text_start_line);
    let hi = line_hi.saturating_sub(text_start_line).min(lines.len() - 1);
    lines[lo..=hi].concat()
}

fn byte_offset_for_line_col(text: &str, line: usize, col: usize) -> Option<usize> {
    let mut byte = 0usize;
    for (idx, l) in split_lines(text).iter().enumerate() {
        if idx + 1 == line {
            let line_text = l.strip_suffix('\n').unwrap_or(l);
            return (col <= line_text.len()).then_some(byte + col);
        }
        byte += l.len();
    }
    None
}

fn byte_range_for_span(text: &str, span: proc_macro2::Span) -> Option<(usize, usize)> {
    let start = span.start();
    let end = span.end();
    let lo = byte_offset_for_line_col(text, start.line, start.column)?;
    let hi = byte_offset_for_line_col(text, end.line, end.column)?;
    (lo < hi).then_some((lo, hi))
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

/// Generic line-based reordering. `text` is a chunk of source whose
/// first line is `text_start_line`. `entries` are the items to sort
/// (each carries its source line range and a name to group/sort on).
///
/// Returns the reordered text. Lines outside the entries' ranges
/// (header, closing brace, comments before/after the field block) are
/// preserved.
///
/// Field-like callers keep existing blank lines with the field that
/// follows them but do not synthesize new blank lines between groups.
#[derive(Clone, Copy)]
struct LineSortOptions {
    include_leading_blank_lines: bool,
    insert_blank_lines_between_groups: bool,
}

impl Default for LineSortOptions {
    fn default() -> Self {
        Self {
            include_leading_blank_lines: false,
            insert_blank_lines_between_groups: true,
        }
    }
}

fn sort_top_level<I>(text: &str, text_start_line: usize, entries: I) -> Option<String>
where
    I: IntoIterator<Item = SortableLines>,
{
    sort_top_level_with_options(text, text_start_line, entries, LineSortOptions::default())
}

fn sort_field_like_top_level<I>(text: &str, text_start_line: usize, entries: I) -> Option<String>
where
    I: IntoIterator<Item = SortableLines>,
{
    sort_top_level_with_options(
        text,
        text_start_line,
        entries,
        LineSortOptions {
            include_leading_blank_lines: true,
            insert_blank_lines_between_groups: false,
        },
    )
}

fn sort_top_level_with_options<I>(
    text: &str,
    text_start_line: usize,
    entries: I,
    options: LineSortOptions,
) -> Option<String>
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
    // We *expand* each entry's line range to also cover comment lines
    // that immediately precede it (so `///` doc comments travel with
    // the field). Some callers also opt into preserving immediately
    // preceding blank lines as leading trivia. The expansion stops
    // when we hit either:
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
        // Expand backwards over preceding `///`, `//`, `#[...]`, and
        // optionally blank lines that look like part of this entry's
        // leading trivia.
        let mut start = first_idx;
        while start > prev_end.map(|p| p + 1).unwrap_or(0) {
            let prev = lines[start - 1].trim_start();
            if prev.starts_with("///")
                || prev.starts_with("//!")
                || prev.starts_with("#[")
                || prev.starts_with("//")
                || (options.include_leading_blank_lines && prev.trim().is_empty())
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
    // of each entry's slice. Some callers insert an additional blank
    // line between groups.
    let mut header: String = lines[..first_field_line].concat();
    let footer: String = lines[last_field_line + 1..].concat();

    // Group by (bucket, prefix). HashMap<key, group_idx> picks the
    // bucket in O(1); we keep a parallel Vec to preserve source-order
    // first-appearance among groups. Per-group totals (bucket, sum,
    // count) are tracked alongside so the cross-group sort below
    // doesn't need to rescan members.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut group_totals: Vec<(u8, usize, usize)> = Vec::new();
    let mut by_key: std::collections::HashMap<(u8, String), usize> =
        std::collections::HashMap::with_capacity(ranges.len());
    for (idx, (_, _, name, bucket)) in ranges.iter().enumerate() {
        let key = (*bucket, prefix_of(name).to_string());
        let name_len = name.len();
        if let Some(&gi) = by_key.get(&key) {
            groups[gi].push(idx);
            group_totals[gi].1 += name_len;
            group_totals[gi].2 += 1;
        } else {
            let gi = groups.len();
            by_key.insert(key, gi);
            groups.push(vec![idx]);
            group_totals.push((*bucket, name_len, 1));
        }
    }
    // Within each group: stable-sort by name length ascending.
    for g in &mut groups {
        g.sort_by_key(|&i| ranges[i].2.len());
    }
    // Between groups: bucket first (lower bucket sorts first —
    // implements "all sync `fn` before all `async fn`" inside impl /
    // trait bodies), then stable-sort by mean name length within the
    // same bucket. Ties preserve source order. We avoid floats by
    // comparing `sum_a * count_b` vs `sum_b * count_a`.
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| {
        let (bucket_a, sum_a, count_a) = group_totals[a];
        let (bucket_b, sum_b, count_b) = group_totals[b];
        if bucket_a != bucket_b {
            return bucket_a.cmp(&bucket_b);
        }
        (sum_a * count_b).cmp(&(sum_b * count_a))
    });
    let groups: Vec<Vec<usize>> = order
        .into_iter()
        .map(|i| std::mem::take(&mut groups[i]))
        .collect();

    // Emit: each entry's slice (which already ends with \n). Some
    // callers insert a blank line between groups; field-like and
    // function-parameter callers preserve existing separators without
    // adding new ones.
    let mut body = String::new();
    for (gi, g) in groups.iter().enumerate() {
        if options.insert_blank_lines_between_groups && gi > 0 {
            body.push('\n');
        }
        for (entry_idx, &i) in g.iter().enumerate() {
            let (lo, hi, _, _) = &ranges[i];
            let mut line_lo = *lo;
            if gi == 0 && entry_idx == 0 && options.include_leading_blank_lines {
                while line_lo <= *hi && lines[line_lo].trim().is_empty() {
                    line_lo += 1;
                }
            }
            for line in &lines[line_lo..=*hi] {
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

/// Reorder named fields in struct-literal expressions (`S { ... }`,
/// `U { ... }`, `E::V { ... }`) using the same grouping rule as
/// declaration fields. The caller passes one complete item body; we
/// re-parse after each successful rewrite so nested literals are handled
/// from the inside out without stale spans.
pub(crate) fn reorder_expr_structs_in_item_text(body_text: &str) -> Option<String> {
    let mut out = body_text.to_string();
    let mut changed = false;

    loop {
        let parsed: Item = syn::parse_str(&out).ok()?;
        let mut collector = ExprStructCollector::default();
        collector.visit_item(&parsed);
        collector.exprs.sort_by_key(|expr| {
            let start = expr.span().start();
            std::cmp::Reverse((start.line, start.column))
        });

        let mut rewrote_one = false;
        for expr in collector.exprs {
            if let Some(rewritten) = rewrite_expr_struct(&out, 1, &expr) {
                if rewritten != out {
                    out = rewritten;
                    changed = true;
                    rewrote_one = true;
                    break;
                }
            }
        }
        if !rewrote_one {
            break;
        }
    }

    changed.then_some(out)
}

#[derive(Default)]
struct ExprStructCollector {
    exprs: Vec<ExprStruct>,
}

impl<'ast> Visit<'ast> for ExprStructCollector {
    fn visit_expr_struct(&mut self, node: &'ast ExprStruct) {
        self.exprs.push(node.clone());
        visit::visit_expr_struct(self, node);
    }
}

pub(crate) fn reorder_single_line_lists_in_item_text(body_text: &str) -> Option<String> {
    let mut out = body_text.to_string();
    let mut changed = false;

    loop {
        let parsed: Item = syn::parse_str(&out).ok()?;
        let mut lists = single_line_lists_for_item(&out, &parsed);
        let mut collector = SingleLineListCollector::new(&out);
        collector.visit_item(&parsed);
        lists.extend(collector.lists);
        lists.sort_by_key(|entries| std::cmp::Reverse(entries.first().map(|e| e.lo).unwrap_or(0)));

        let mut rewrote_one = false;
        for entries in lists {
            if let Some(rewritten) = rewrite_single_line_list(&out, &entries) {
                out = rewritten;
                changed = true;
                rewrote_one = true;
                break;
            }
        }

        if !rewrote_one {
            break;
        }
    }

    changed.then_some(out)
}

struct SingleLineListCollector<'a> {
    lists: Vec<Vec<SortableSpan>>,
    text: &'a str,
}

impl<'a> SingleLineListCollector<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            lists: Vec::new(),
            text,
        }
    }
}

impl<'ast> Visit<'ast> for SingleLineListCollector<'_> {
    fn visit_expr_struct(&mut self, node: &'ast ExprStruct) {
        if let Some(entries) = expr_struct_single_line_entries(self.text, node) {
            self.lists.push(entries);
        }
        visit::visit_expr_struct(self, node);
    }
}

pub(crate) fn reorder_fn_args_in_item_text(body_text: &str) -> Option<String> {
    let mut out = body_text.to_string();
    let mut changed = false;

    loop {
        let parsed: Item = syn::parse_str(&out).ok()?;
        let mut collector = SignatureCollector::default();
        collector.visit_item(&parsed);
        collector.sigs.sort_by_key(|sig| {
            let start = sig.span().start();
            std::cmp::Reverse((start.line, start.column))
        });

        let mut rewrote_one = false;
        for sig in collector.sigs {
            if let Some(rewritten) = rewrite_signature_args(&out, &sig) {
                if rewritten != out {
                    out = rewritten;
                    changed = true;
                    rewrote_one = true;
                    break;
                }
            }
        }

        if !rewrote_one {
            break;
        }
    }

    changed.then_some(out)
}

#[derive(Default)]
struct SignatureCollector {
    sigs: Vec<Signature>,
}

impl<'ast> Visit<'ast> for SignatureCollector {
    fn visit_signature(&mut self, node: &'ast Signature) {
        self.sigs.push(node.clone());
        visit::visit_signature(self, node);
    }
}

fn container_skips(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        if a.path().is_ident("repr") {
            return true;
        }
        if a.path().is_ident("derive") {
            return derive_list_pins_order(a);
        }
        if a.path().is_ident("cfg_attr") {
            // `#[cfg_attr(<cfg>, derive(Ord, PartialOrd), ...)]`: a
            // conditional derive can still pin field order under the
            // matching cfg. We don't evaluate the cfg expression — if
            // the attribute's token stream syntactically contains a
            // `derive(...)` mentioning Ord / PartialOrd, skip
            // reordering. Token-stream substring is safe here because
            // both names are reserved trait identifiers; a field or
            // value literally spelled `Ord` would still trigger a
            // (harmless) skip.
            return cfg_attr_pins_order(a);
        }
        false
    })
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
    sort_field_like_top_level(
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

fn rewrite_union(u: &ItemUnion, body_text: &str, start_line: usize) -> Option<String> {
    if container_skips(&u.attrs) {
        return None;
    }
    let pin_last = has_unsized_generic(&u.generics);
    rewrite_named_fields_inplace(body_text, start_line, &u.fields, pin_last)
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

fn rewrite_expr_struct(text: &str, text_start_line: usize, expr: &ExprStruct) -> Option<String> {
    if expr.fields.len() < 2 {
        return None;
    }
    if expr
        .fields
        .iter()
        .any(|f| !matches!(f.member, Member::Named(_)))
    {
        return None;
    }
    sort_field_like_top_level(
        text,
        text_start_line,
        expr.fields.iter().map(|f| {
            let name = match &f.member {
                Member::Named(ident) => ident.to_string(),
                Member::Unnamed(_) => String::new(),
            };
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
                bucket: 0,
            }
        }),
    )
}

fn single_line_lists_for_item(text: &str, item: &Item) -> Vec<Vec<SortableSpan>> {
    let mut out = Vec::new();
    match item {
        Item::Struct(s) if !container_skips(&s.attrs) => {
            if let Fields::Named(named) = &s.fields {
                if let Some(entries) = fields_named_single_line_entries(text, named) {
                    out.push(entries);
                }
            }
        }
        Item::Union(u) if !container_skips(&u.attrs) => {
            if let Some(entries) = fields_named_single_line_entries(text, &u.fields) {
                out.push(entries);
            }
        }
        Item::Enum(e)
            if !container_skips(&e.attrs)
                && !e.variants.iter().any(|v| v.discriminant.is_some()) =>
        {
            if let Some(entries) = enum_variant_single_line_entries(text, e) {
                out.push(entries);
            }
            for v in &e.variants {
                if let Fields::Named(named) = &v.fields {
                    if let Some(entries) = fields_named_single_line_entries(text, named) {
                        out.push(entries);
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn fields_named_single_line_entries(text: &str, named: &FieldsNamed) -> Option<Vec<SortableSpan>> {
    if named.named.len() < 2 || named.named.iter().any(|f| !f.attrs.is_empty()) {
        return None;
    }
    let mut entries = Vec::with_capacity(named.named.len());
    for f in &named.named {
        let name = f.ident.as_ref()?.to_string();
        let (lo, hi) = byte_range_for_span(text, f.span())?;
        entries.push(SortableSpan { name, lo, hi });
    }
    single_line_entries(entries)
}

fn expr_struct_single_line_entries(text: &str, expr: &ExprStruct) -> Option<Vec<SortableSpan>> {
    if expr.fields.len() < 2 {
        return None;
    }
    let mut entries = Vec::with_capacity(expr.fields.len());
    for f in &expr.fields {
        if !f.attrs.is_empty() {
            return None;
        }
        let Member::Named(ident) = &f.member else {
            return None;
        };
        let (lo, hi) = byte_range_for_span(text, f.span())?;
        entries.push(SortableSpan {
            name: ident.to_string(),
            lo,
            hi,
        });
    }
    single_line_entries(entries)
}

fn enum_variant_single_line_entries(text: &str, e: &ItemEnum) -> Option<Vec<SortableSpan>> {
    if e.variants.len() < 2 || e.variants.iter().any(|v| !v.attrs.is_empty()) {
        return None;
    }
    let mut entries = Vec::with_capacity(e.variants.len());
    for v in &e.variants {
        let (lo, hi) = byte_range_for_span(text, v.span())?;
        entries.push(SortableSpan {
            name: v.ident.to_string(),
            lo,
            hi,
        });
    }
    single_line_entries(entries)
}

fn rewrite_signature_args(text: &str, sig: &Signature) -> Option<String> {
    let entries = signature_arg_entries(text, sig)?;
    rewrite_single_line_list(text, &entries).or_else(|| {
        sort_top_level_with_options(
            text,
            1,
            entries.iter().map(|e| {
                let span = &text[e.lo..e.hi];
                let first_line = 1 + text[..e.lo].bytes().filter(|b| *b == b'\n').count();
                let last_line = first_line + span.bytes().filter(|b| *b == b'\n').count();
                SortableLines {
                    name: e.name.clone(),
                    bucket: 0,
                    first_line,
                    last_line,
                }
            }),
            LineSortOptions {
                include_leading_blank_lines: true,
                insert_blank_lines_between_groups: false,
            },
        )
    })
}

fn signature_arg_entries(text: &str, sig: &Signature) -> Option<Vec<SortableSpan>> {
    if sig.inputs.len() < 2 {
        return None;
    }
    let mut inputs = sig.inputs.iter();
    if matches!(inputs.clone().next(), Some(FnArg::Receiver(_))) {
        inputs.next();
    }

    let mut entries = Vec::with_capacity(sig.inputs.len());
    for input in inputs {
        let FnArg::Typed(pat) = input else {
            return None;
        };
        if !pat.attrs.is_empty() {
            return None;
        }
        let Pat::Ident(ident) = pat.pat.as_ref() else {
            return None;
        };
        let (lo, hi) = byte_range_for_span(text, input.span())?;
        entries.push(SortableSpan {
            name: ident.ident.to_string(),
            lo,
            hi,
        });
    }
    single_line_entries(entries)
}

fn single_line_entries(mut entries: Vec<SortableSpan>) -> Option<Vec<SortableSpan>> {
    if entries.len() < 2 {
        return None;
    }
    entries.sort_by_key(|e| e.lo);
    let mut prev_hi = 0usize;
    for e in &entries {
        if e.lo < prev_hi {
            return None;
        }
        prev_hi = e.hi;
    }
    // Byte offsets are not enough to prove same-line; ask the spans'
    // source text slice range not to contain a newline across the whole
    // list. This keeps the byte-level rewrite scoped to one physical line.
    if entries.iter().any(|e| e.name.is_empty()) {
        return None;
    }
    Some(entries)
}

fn rewrite_single_line_list(text: &str, entries: &[SortableSpan]) -> Option<String> {
    if entries.len() < 2 {
        return None;
    }
    let mut entries = entries.to_vec();
    entries.sort_by_key(|e| e.lo);
    let lo = entries.first()?.lo;
    let hi = entries.last()?.hi;
    if text.get(lo..hi)?.contains('\n') {
        return None;
    }

    let keys = compute_group_keys(
        entries
            .iter()
            .enumerate()
            .map(|(idx, e)| (idx, e.name.as_str())),
    );
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|idx| (keys[idx], *idx));

    let replacement = order
        .iter()
        .map(|idx| text[entries[*idx].lo..entries[*idx].hi].trim())
        .collect::<Vec<_>>()
        .join(", ");
    if replacement == text[lo..hi] {
        return None;
    }

    let mut out = String::with_capacity(text.len() - (hi - lo) + replacement.len());
    out.push_str(&text[..lo]);
    out.push_str(&replacement);
    out.push_str(&text[hi..]);
    Some(out)
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
    sort_field_like_top_level(
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
                bucket: if pin_last && i == last_idx {
                    PIN_LAST
                } else {
                    0
                },
            }
        }),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GroupSortKey {
    mean_sum: u64,

    mean_count: u32,

    group_order: usize,

    name_len: u32,
}

impl GroupSortKey {
    pub(crate) fn source_order(idx: usize) -> Self {
        Self {
            mean_sum: 0,
            mean_count: 1,
            group_order: idx,
            name_len: 0,
        }
    }
}

impl Ord for GroupSortKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.mean_sum as u128 * other.mean_count as u128)
            .cmp(&(other.mean_sum as u128 * self.mean_count as u128))
            .then_with(|| self.group_order.cmp(&other.group_order))
            .then_with(|| self.name_len.cmp(&other.name_len))
    }
}

impl PartialOrd for GroupSortKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Compute a per-item group sort key for each `(idx, name)` pair, where:
/// - All items sharing the same `prefix_of(name)` form one group.
/// - Groups sort by exact mean name length using cross multiplication.
/// - Ties between groups preserve the first source appearance of the
///   group, so a same-mean group stays contiguous.
/// - Within a group, shorter names sort first; ties preserve source order.
pub(crate) fn compute_group_keys<'a, I>(items: I) -> std::collections::HashMap<usize, GroupSortKey>
where
    I: IntoIterator<Item = (usize, &'a str)>,
{
    let items: Vec<(usize, &'a str)> = items.into_iter().collect();
    let mut groups: std::collections::HashMap<&'a str, (u64, u32, usize)> =
        std::collections::HashMap::new();
    for (_, name) in &items {
        let next_order = groups.len();
        let entry = groups
            .entry(prefix_of(name))
            .or_insert((0u64, 0u32, next_order));
        entry.0 += name.len() as u64;
        entry.1 += 1;
    }
    let mut out = std::collections::HashMap::with_capacity(items.len());
    for (idx, name) in items {
        let pfx = prefix_of(name);
        let (sum, count, group_order) = groups[pfx];
        out.insert(
            idx,
            GroupSortKey {
                mean_sum: sum,
                mean_count: count,
                group_order,
                name_len: name.len() as u32,
            },
        );
    }
    out
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
    generics.where_clause.as_ref().is_some_and(|w| {
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

fn cfg_attr_pins_order(attr: &Attribute) -> bool {
    let Ok(list) = attr.meta.require_list() else {
        return false;
    };
    let tokens = list.tokens.to_string();
    if !tokens.contains("derive") {
        return false;
    }
    // Whole-word match on Ord / PartialOrd to avoid matching identifiers
    // that happen to contain "Ord" as a substring.
    tokens
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|w| w == "Ord" || w == "PartialOrd")
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

fn derive_list_pins_order(attr: &Attribute) -> bool {
    let mut found = false;
    let _ = attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("PartialOrd") || meta.path.is_ident("Ord") {
            found = true;
        }
        Ok(())
    });
    found
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
