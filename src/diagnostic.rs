//! Output formatting that mirrors rustfmt / cargo fmt.
//!
//! * Parse errors are rendered in rustc-style with `error:` headline,
//!   `--> path:line:col` source pointer, and a caret under the offending
//!   token.
//! * `--check` diffs are emitted as rustfmt-style hunks:
//!   `Diff in <path> at line <N>:` followed by `+` / `-` / ` ` lines.
//! * Fatal errors get an `error:` prefix.
//!
//! Colour output is gated on `Color`. ANSI SGR codes used:
//!   31 = red, 32 = green, 36 = cyan, 1 = bold, 0 = reset.

use std::io::{self, IsTerminal, Write};
use std::path::Path;

use similar::{ChangeTag, TextDiff};

const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const BOLD: &str = "\x1b[1m";
const BOLD_RED: &str = "\x1b[1;31m";
const BOLD_CYAN: &str = "\x1b[1;36m";
const RESET: &str = "\x1b[0m";

/// Whether to emit ANSI colour escapes. Mirrors rustfmt's `--color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// Always emit colour, regardless of tty / `NO_COLOR`.
    Always,
    /// Never emit colour.
    Never,
    /// Emit colour only when the sink is a tty and `NO_COLOR` is unset.
    Auto,
}

impl Color {
    /// Resolve `Auto` by querying `stderr` (where diffs and errors land).
    pub fn enabled_for_stderr(self) -> bool {
        match self {
            Color::Always => true,
            Color::Never => false,
            Color::Auto => io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }
}

/// Render a `syn::Error` in rustc style.
///
/// The output looks like:
/// ```text
/// error: expected `{`
///   --> src/lib.rs:10:5
///    |
/// 10 |     foo()
///    |     ^
/// ```
pub fn format_parse_error(path: &Path, source: &str, err: &syn::Error) -> String {
    format_parse_error_colored(path, source, err, false)
}

pub fn format_parse_error_colored(
    path: &Path,
    source: &str,
    err: &syn::Error,
    color: bool,
) -> String {
    let mut out = String::new();
    let span = err.span();
    let start = span.start();
    let line = start.line.max(1);
    let col = start.column;
    let msg = err.to_string();

    let (err_p, err_s, bold_p, bold_s, gutter_p, gutter_s, caret_p, caret_s) = if color {
        (
            BOLD_RED, RESET, BOLD, RESET, BOLD_CYAN, RESET, BOLD_RED, RESET,
        )
    } else {
        ("", "", "", "", "", "", "", "")
    };

    out.push_str(&format!("{err_p}error{err_s}{bold_p}: {msg}{bold_s}\n"));
    out.push_str(&format!(
        "{gutter_p}  -->{gutter_s} {}:{}:{}\n",
        path.display(),
        line,
        col + 1
    ));

    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let line_str = lines.get(line.saturating_sub(1)).copied().unwrap_or("");
    let line_str = line_str.trim_end_matches(['\r', '\n']);

    let gutter_width = line.to_string().len();
    out.push_str(&format!(
        "{:width$} {gutter_p}|{gutter_s}\n",
        "",
        width = gutter_width,
    ));
    out.push_str(&format!(
        "{gutter_p}{:>width$} |{gutter_s} {}\n",
        line,
        line_str,
        width = gutter_width,
    ));
    out.push_str(&format!(
        "{:width$} {gutter_p}|{gutter_s} {}{caret_p}^{caret_s}\n",
        "",
        " ".repeat(col),
        width = gutter_width,
    ));
    out
}

/// Write a unified diff between `original` and `reordered`, prefixed with
/// `Diff in <path> at line <N>:` headers — rustfmt's `--check` format.
pub fn write_diff(w: impl Write, path: &Path, original: &str, reordered: &str) -> io::Result<()> {
    write_diff_colored(w, path, original, reordered, false)
}

/// Same as [`write_diff`] but emits ANSI colour escapes for `-` (red),
/// `+` (green), and the `Diff in ...` header (cyan) when `color` is true.
pub fn write_diff_colored(
    mut w: impl Write,
    path: &Path,
    original: &str,
    reordered: &str,
    color: bool,
) -> io::Result<()> {
    let diff = TextDiff::from_lines(original, reordered);
    for group in diff.grouped_ops(3) {
        if group.is_empty() {
            continue;
        }
        let first = group.first().unwrap();
        let old_start = first.old_range().start;
        if color {
            writeln!(
                w,
                "{CYAN}Diff in {} at line {}:{RESET}",
                path.display(),
                old_start + 1
            )?;
        } else {
            writeln!(w, "Diff in {} at line {}:", path.display(), old_start + 1)?;
        }
        for op in &group {
            for change in diff.iter_changes(op) {
                let mut value = change.value().to_string();
                if value.ends_with('\n') {
                    value.pop();
                    if value.ends_with('\r') {
                        value.pop();
                    }
                }
                match change.tag() {
                    ChangeTag::Equal => writeln!(w, " {value}")?,
                    ChangeTag::Delete => {
                        if color {
                            writeln!(w, "{RED}-{value}{RESET}")?;
                        } else {
                            writeln!(w, "-{value}")?;
                        }
                    }
                    ChangeTag::Insert => {
                        if color {
                            writeln!(w, "{GREEN}+{value}{RESET}")?;
                        } else {
                            writeln!(w, "+{value}")?;
                        }
                    }
                }
            }
        }
        writeln!(w)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_has_rustc_shape() {
        let src = "fn main() {\n    let x = ;\n}\n";
        let err = syn::parse_file(src).unwrap_err();
        let out = format_parse_error(Path::new("foo.rs"), src, &err);
        assert!(out.starts_with("error: "), "{out}");
        assert!(out.contains("--> foo.rs:"), "{out}");
        assert!(out.contains(" | "), "{out}");
        assert!(out.contains("^"), "{out}");
    }

    #[test]
    fn diff_has_rustfmt_header() {
        let a = "use a;\nfn foo() {}\n";
        let b = "use a;\npub fn foo() {}\n";
        let mut buf = Vec::new();
        write_diff(&mut buf, Path::new("x.rs"), a, b).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Diff in x.rs at line "), "{s}");
        assert!(s.contains("-fn foo() {}"), "{s}");
        assert!(s.contains("+pub fn foo() {}"), "{s}");
        // Plain mode: no ANSI escapes.
        assert!(
            !s.contains("\x1b["),
            "plain diff must not contain ANSI: {s:?}"
        );
    }

    #[test]
    fn coloured_diff_wraps_minus_and_plus_lines() {
        let a = "use a;\nfn foo() {}\n";
        let b = "use a;\npub fn foo() {}\n";
        let mut buf = Vec::new();
        write_diff_colored(&mut buf, Path::new("x.rs"), a, b, true).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains("\x1b[31m-fn foo() {}\x1b[0m"),
            "red `-` line: {s:?}"
        );
        assert!(
            s.contains("\x1b[32m+pub fn foo() {}\x1b[0m"),
            "green `+` line: {s:?}"
        );
        assert!(s.contains("\x1b[36mDiff in x.rs"), "cyan header: {s:?}");
    }

    #[test]
    fn coloured_parse_error_wraps_error_and_caret() {
        let src = "fn main() {\n    let x = ;\n}\n";
        let err = syn::parse_file(src).unwrap_err();
        let out = format_parse_error_colored(Path::new("foo.rs"), src, &err, true);
        assert!(out.starts_with("\x1b[1;31merror\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[1;36m  -->\x1b[0m"), "{out:?}");
        assert!(out.contains("\x1b[1;31m^\x1b[0m"), "{out:?}");
    }
}
