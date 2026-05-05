//! Cross-file macro constraint: when reordering a parent file (lib.rs),
//! we open each `mod foo;` declaration's child file and check whether
//! it actually invokes any of our local `macro_rules!` bare. Only those
//! mods need the textual-scope constraint; mods whose child file
//! imports the macro via `use crate::name;` (or doesn't use it at all)
//! impose no position constraint and let the macro sort to its default
//! end-of-file slot.

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

/// Real-world shape: original parent file has the macro declared *before*
/// the `mod foo;` (so it compiles via Rust's textual-scope inheritance).
/// Default category sort would push the macro to the end (Category::Macro
/// weight 92) and the mod near the top (weight 30), breaking the file.
/// The cross-file scan must detect that paths.rs invokes `t!()` bare and
/// keep the macro above the mod in the output.
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

/// Same shape but the child file does `use crate::t;`. Now the macro
/// is reachable through name resolution, no textual scope needed, and
/// the cross-file scan should drop the position constraint — the
/// macro is free to sort to its default end-of-file slot.
#[test]
fn child_using_use_crate_macro_relaxes_constraint() {
    let td = TempDir::new("relaxes");
    td.write(
        "src/paths.rs",
        "use crate::t;\npub fn helper() {\n    t!(\"hi\");\n}\n",
    );
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
        p_mod < p_macro,
        "with `use crate::t;` in paths.rs, the macro is free to sort to its default end-of-file position:\n{out}"
    );
}

/// Child doesn't reference the macro at all → no constraint.
#[test]
fn child_that_does_not_use_the_macro_does_not_constrain() {
    let td = TempDir::new("unused");
    td.write("src/paths.rs", "pub fn helper() {}\n");
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
        p_mod < p_macro,
        "child doesn't use the macro at all → no position constraint:\n{out}"
    );
}

/// `#[path = "..."]` overrides the conventional file lookup. The
/// cross-file scan must follow it.
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

/// `mod absent;` declared but `absent.rs` doesn't exist on disk →
/// cross-file scan can't read it → fall back to the conservative rule
/// (mod was originally after the macro in source, so we preserve that).
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
