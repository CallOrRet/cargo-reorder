//! Line-oriented string utilities used by the reorder pipeline.
//!
//! All functions are 1-indexed when they take "line numbers" (so they
//! match the line numbers `proc_macro2::Span` returns). `split_keep_endings`
//! preserves trailing newlines on each line, so `concat()`-ing slices of
//! its output reconstructs the input verbatim.

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

/// Split `region` at the last blank line. Everything up to and including
/// the last blank line goes to the first half (the "trailing trivia" of
/// whatever came before it); everything after goes to the second half
/// (the "leading trivia" of whatever comes next).
pub(crate) fn split_at_last_blank(region: &str) -> (String, String) {
    if region.is_empty() {
        return (String::new(), String::new());
    }
    let lines = split_keep_endings(region);
    let mut split_idx = 0usize;
    for i in (0..lines.len()).rev() {
        if line_is_blank(lines[i]) {
            split_idx = i + 1;
            break;
        }
    }
    let before: String = lines[..split_idx].concat();
    let after: String = lines[split_idx..].concat();
    (before, after)
}

pub(crate) fn line_is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

pub(crate) fn starts_with_blank_line(s: &str) -> bool {
    s.starts_with('\n') || s.starts_with("\r\n")
}

pub(crate) fn ends_with_blank_line(s: &str) -> bool {
    s.ends_with("\n\n") || s.ends_with("\r\n\r\n")
}
