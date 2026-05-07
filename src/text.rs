//! Line-oriented string utilities used by the reorder pipeline.
//!
//! All functions are 1-indexed when they take "line numbers" (so they
//! match the line numbers `proc_macro2::Span` returns). `split_keep_endings`
//! preserves trailing newlines on each line, so `concat()`-ing slices of
//! its output reconstructs the input verbatim.

/// Concatenate `lines[from-1 ..= to-1]`. Inputs are 1-indexed and clamped
/// against the slice bounds; out-of-range or `from > to` returns "".
pub(crate) fn take_lines(lines: &[&str], from: usize, to: usize) -> String {
    if from == 0 || from > to || from > lines.len() {
        return String::new();
    }
    let lo = from - 1;
    let hi = to.min(lines.len());
    lines[lo..hi].concat()
}

pub(crate) fn line_is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// Split `s` at every `\n`, keeping the newline as the last byte of each
/// returned slice. The final slice has no newline if `s` doesn't end in
/// one. `concat()`-ing the result reconstructs `s` byte-for-byte.
pub(crate) fn split_keep_endings(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            out.push(&s[start..=i]);
            start = i + 1;
        }
        i += 1;
    }
    if start < bytes.len() {
        out.push(&s[start..]);
    }
    out
}

/// Split `region` at the last blank line. Everything up to and including
/// the last blank line goes to the first half (the "trailing trivia" of
/// whatever came before it); everything after goes to the second half
/// (the "leading trivia" of whatever comes next).
pub(crate) fn split_at_last_blank(region: &str) -> (String, String) {
    if region.is_empty() {
        return (String::new(), String::new());
    }
    let lines = split_keep_endings(region);
    // Track whether each line *starts* inside an unclosed `/* ... */`
    // block comment. We must never split between two halves of a block
    // comment — the `/*` and the `*/` would end up in different items'
    // trivia, and a reorder could insert real code between them, eating
    // it into the comment. (Real-world repro: nom's src/internal.rs.)
    let mut in_block = vec![false; lines.len()];
    let mut depth = 0i32;
    for (i, line) in lines.iter().enumerate() {
        in_block[i] = depth > 0;
        let bytes = line.as_bytes();
        let mut j = 0;
        while j + 1 < bytes.len() {
            if depth == 0 && bytes[j] == b'/' && bytes[j + 1] == b'/' {
                break;
            }
            if bytes[j] == b'/' && bytes[j + 1] == b'*' {
                depth += 1;
                j += 2;
                continue;
            }
            if depth > 0 && bytes[j] == b'*' && bytes[j + 1] == b'/' {
                depth -= 1;
                j += 2;
                continue;
            }
            j += 1;
        }
    }
    let mut split_idx = 0usize;
    for i in (0..lines.len()).rev() {
        if line_is_blank(lines[i]) && !in_block[i] {
            split_idx = i + 1;
            break;
        }
    }
    let before: String = lines[..split_idx].concat();
    let after: String = lines[split_idx..].concat();
    (before, after)
}

pub(crate) fn ends_with_blank_line(s: &str) -> bool {
    s.ends_with("\n\n") || s.ends_with("\r\n\r\n")
}

pub(crate) fn starts_with_blank_line(s: &str) -> bool {
    s.starts_with('\n') || s.starts_with("\r\n")
}

/// If `gap` between two items looks like
/// ```text
/// [one or more blank lines]
/// [one or more `//` comment lines]   <-- but not `///` or `//!` doc comments
/// [one or more blank lines]
/// ```
/// return `Some((comment_text_with_trailing_blank, residual_gap))`. The
/// caller treats the comment as a "fence" anchored to its original spot
/// (items above and below the fence are forbidden from reordering across
/// it) and feeds `residual_gap` to [`split_at_last_blank`] for the
/// trailing/leading split between the two items.
///
/// `///` outer-doc and `//!` inner-doc comments are excluded — those
/// usually belong to a sibling item and would normally be parsed as
/// attributes by `syn` (so they shouldn't show up here, but we guard
/// anyway).
pub(crate) fn extract_floating_comment(gap: &str) -> Option<(String, String)> {
    let lines = split_keep_endings(gap);
    if lines.is_empty() {
        return None;
    }
    let first_non_blank = lines.iter().position(|l| !line_is_blank(l))?;
    if first_non_blank == 0 {
        // Comment must be preceded by at least one blank line.
        return None;
    }
    let mut after = first_non_blank;
    for (i, line) in lines.iter().enumerate().skip(first_non_blank) {
        if line_is_blank(line) {
            after = i;
            break;
        }
        let t = line.trim_start();
        if !t.starts_with("//") || t.starts_with("///") || t.starts_with("//!") {
            return None;
        }
        after = i + 1;
    }
    // `after` is now the index of the first blank line after the comment
    // block, or lines.len() if the gap ended with the comment.
    if after >= lines.len() || !line_is_blank(lines[after]) {
        return None;
    }
    // Comment text owns one trailing blank to keep the visual gap
    // around it after we splice it back in.
    let comment_text: String = lines[first_non_blank..=after].concat();
    let mut residual = String::new();
    residual.push_str(&lines[..first_non_blank].concat());
    residual.push_str(&lines[after + 1..].concat());
    Some((comment_text, residual))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floating_comment_basic() {
        let (c, r) = extract_floating_comment("\n// hi\n\n").unwrap();
        assert_eq!(c, "// hi\n\n");
        assert_eq!(r, "\n");
    }

    #[test]
    fn floating_comment_multi_line() {
        let (c, r) = extract_floating_comment("\n// a\n// b\n// c\n\n").unwrap();
        assert_eq!(c, "// a\n// b\n// c\n\n");
        assert_eq!(r, "\n");
    }

    #[test]
    fn floating_comment_extra_blanks_both_sides() {
        // Multiple blanks before/after still detect; only the immediate
        // sandwich rule matters and the comment owns one trailing blank.
        let (c, r) = extract_floating_comment("\n\n// hi\n\n\n").unwrap();
        assert_eq!(c, "// hi\n\n");
        assert_eq!(r, "\n\n\n");
    }

    #[test]
    fn floating_comment_rejects_no_leading_blank() {
        assert!(extract_floating_comment("// hi\n\n").is_none());
    }

    #[test]
    fn floating_comment_rejects_no_trailing_blank() {
        // Comment with nothing after it (gap ends with the comment) —
        // belongs to the next item's leading, not a fence.
        assert!(extract_floating_comment("\n// hi\n").is_none());
    }

    #[test]
    fn floating_comment_rejects_doc_comment() {
        assert!(extract_floating_comment("\n/// hi\n\n").is_none());
        assert!(extract_floating_comment("\n//! hi\n\n").is_none());
    }

    #[test]
    fn floating_comment_rejects_block_comment() {
        // `/* ... */` is intentionally not handled — these are rare as
        // floating dividers and the line-based heuristic skips them.
        assert!(extract_floating_comment("\n/* hi */\n\n").is_none());
    }

    #[test]
    fn floating_comment_rejects_empty() {
        assert!(extract_floating_comment("").is_none());
        assert!(extract_floating_comment("\n").is_none());
        assert!(extract_floating_comment("\n\n").is_none());
    }

    #[test]
    fn floating_comment_rejects_non_comment_content() {
        // A real item line breaks the sandwich.
        assert!(extract_floating_comment("\nfn x() {}\n\n").is_none());
    }

    #[test]
    fn split_at_last_blank_basic() {
        assert_eq!(split_at_last_blank(""), (String::new(), String::new()));
        assert_eq!(split_at_last_blank("\n"), ("\n".to_string(), String::new()));
        assert_eq!(
            split_at_last_blank("a\n\nb\n"),
            ("a\n\n".to_string(), "b\n".to_string())
        );
    }

    #[test]
    fn split_keep_endings_round_trip() {
        for s in ["", "a", "a\n", "a\nb", "a\nb\n", "\n\n", "x\r\ny"] {
            assert_eq!(split_keep_endings(s).concat(), s);
        }
    }

    #[test]
    fn take_lines_clamping() {
        let lines = split_keep_endings("a\nb\nc\n");
        assert_eq!(take_lines(&lines, 1, 1), "a\n");
        assert_eq!(take_lines(&lines, 1, 3), "a\nb\nc\n");
        assert_eq!(take_lines(&lines, 2, 100), "b\nc\n"); // clamps high
        assert_eq!(take_lines(&lines, 0, 2), ""); // 1-indexed; 0 means empty
        assert_eq!(take_lines(&lines, 5, 6), ""); // out of range
    }
}
