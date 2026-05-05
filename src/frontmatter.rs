//! RFC 3502 cargo-script frontmatter detection.
//!
//! A cargo-script file is shaped:
//!
//! ```text
//! #!/usr/bin/env -S cargo +nightly -Zscript     (optional shebang)
//! ---<info>?                                    (opener: 3+ dashes)
//! [package]
//! edition = "2024"
//! ---<info>?                                    (closer: same dash count)
//!
//! fn main() { /* Rust body */ }
//! ```
//!
//! `syn` chokes on the `---` lines, so we slice them off before parsing
//! and stitch them back unchanged after reordering.

/// Returns `Some((prefix, body))` if `source` starts with cargo-script
/// frontmatter — `prefix` is everything up to and including the closing
/// fence (kept verbatim), `body` is the Rust that follows.
pub(crate) fn split(source: &str) -> Option<(String, String)> {
    let mut cursor = 0;

    if source.starts_with("#!") {
        cursor = source.find('\n')? + 1;
    }
    // Skip blank lines between shebang and opener.
    while cursor < source.len() {
        let rest = &source[cursor..];
        if let Some(stripped) = rest.strip_prefix("\r\n") {
            cursor = source.len() - stripped.len();
        } else if let Some(stripped) = rest.strip_prefix('\n') {
            cursor = source.len() - stripped.len();
        } else {
            break;
        }
    }

    let after = &source[cursor..];
    if !after.starts_with("---") {
        return None;
    }
    let dashes = after.bytes().take_while(|&b| b == b'-').count();
    if dashes < 3 {
        return None;
    }
    let opener_end = cursor + after.find('\n')? + 1;

    let dash_run = "-".repeat(dashes);
    let mut line_start = opener_end;
    while line_start < source.len() {
        let line_end = source[line_start..]
            .find('\n')
            .map(|n| line_start + n + 1)
            .unwrap_or(source.len());
        let line = &source[line_start..line_end];
        // Closer: same dash count, no extra dash glued on.
        if line.starts_with(&dash_run) && !line[dashes..].starts_with('-') {
            return Some((
                source[..line_end].to_string(),
                source[line_end..].to_string(),
            ));
        }
        line_start = line_end;
    }
    None
}
