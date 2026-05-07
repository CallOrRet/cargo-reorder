//! Cross-file behaviour around `mod foo;` declarations: with macro
//! items now treated as hard barriers (see `src/macros.rs`), the
//! parent file no longer opens child files to look for callers — the
//! barrier alone keeps every `macro_rules!` pinned in source position.
//! These tests cover the leftover surface area:
//!   * `mod foo;` lookup still locates `src/foo.rs` / `src/foo/mod.rs`
//!   * `#[path = "..."]` overrides are honoured
//!   * missing child files don't crash discovery

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cargo_reorder::reorder_source_with_path;

static UNIQ: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    dir: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let mut dir = env::temp_dir();
        let n = UNIQ.fetch_add(1, Ordering::Relaxed);
        dir.push(format!(
            "cargo-reorder-xfile-{name}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn write(&self, rel: &str, src: &str) -> PathBuf {
        let p = self.dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, src).unwrap();
        p
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// `#[path = "..."]` overrides the conventional child-file lookup.
/// Reorder still pins the `macro_rules!` at its source position via
/// the barrier rule and `pub mod helper` declared underneath stays
/// below it, regardless of where the child file actually lives.
#[test]
fn path_attribute_on_mod_locates_child_file() {
    let td = TempDir::new("path_attr");
    td.write(
        "src/elsewhere/helper.rs",
        "pub fn helper() {\n    t!(\"hi\");\n}\n",
    );
    let lib = td.write(
        "src/lib.rs",
        r#"#[macro_export]
macro_rules! t {
    ($e:expr) => { let _ = $e; };
}

#[path = "elsewhere/helper.rs"]
pub mod helper;
"#,
    );
    let src = fs::read_to_string(&lib).unwrap();
    let out = reorder_source_with_path(&src, Some(&lib), &Default::default()).unwrap();
    let p_macro = out.find("macro_rules! t").unwrap();
    let p_mod = out.find("pub mod helper").unwrap();
    assert!(
        p_macro < p_mod,
        "macro must precede mod even when child path is overridden via #[path]:\n{out}"
    );
}

/// `mod absent;` declared but `absent.rs` doesn't exist on disk —
/// missing child files are not consulted; the macro-as-barrier rule
/// alone keeps `mod absent;` below the `macro_rules!` declared above
/// it in source.
#[test]
fn missing_child_file_falls_back_to_conservative() {
    let td = TempDir::new("missing");
    let lib = td.write(
        "src/lib.rs",
        r#"#[macro_export]
macro_rules! t {
    ($e:expr) => { let _ = $e; };
}

pub mod absent;
"#,
    );
    let src = fs::read_to_string(&lib).unwrap();
    let out = reorder_source_with_path(&src, Some(&lib), &Default::default()).unwrap();
    let p_macro = out.find("macro_rules! t").unwrap();
    let p_mod = out.find("pub mod absent").unwrap();
    assert!(
        p_macro < p_mod,
        "unreadable child must fall back to conservative — macro before mod:\n{out}"
    );
}
/// Real-world shape: original parent file has the macro declared *before*
/// the `mod foo;` (so it compiles via Rust's textual-scope inheritance).
/// Naive category sort would push the macro to the end
/// (Category::Macro weight 92) and the mod near the top (default
/// weight 10), breaking the file. The macro-as-barrier rule pins the
/// `macro_rules!` in its source position, which keeps the mod above
/// it from sorting down past it — no child-file scan is needed.
#[test]
fn child_with_bare_macro_call_keeps_macro_above_mod() {
    let td = TempDir::new("constrains");
    td.write("src/paths.rs", "pub fn helper() {\n    t!(\"hi\");\n}\n");
    let lib = td.write(
        "src/lib.rs",
        r#"#[macro_export]
macro_rules! t {
    ($e:expr) => { let _ = $e; };
}

pub mod paths;
"#,
    );
    let src = fs::read_to_string(&lib).unwrap();
    let out = reorder_source_with_path(&src, Some(&lib), &Default::default()).unwrap();
    let p_macro = out.find("macro_rules! t").unwrap();
    let p_mod = out.find("pub mod paths").unwrap();
    assert!(
        p_macro < p_mod,
        "macro_rules! t! must precede `mod paths;` because paths.rs invokes t!() bare:\n{out}"
    );
}

