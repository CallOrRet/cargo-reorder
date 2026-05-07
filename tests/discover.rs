//! Integration tests for `cargo fmt`-style file discovery: build a tiny
//! cargo project in a tempdir, run discovery, check the right files are
//! found and the wrong ones are skipped.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use cargo_reorder::discover::{DiscoverOptions, discover};

static UNIQ: AtomicU64 = AtomicU64::new(0);

struct Workspace {
    dir: PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Self {
        let mut dir = env::temp_dir();
        let n = UNIQ.fetch_add(1, Ordering::Relaxed);
        dir.push(format!(
            "cargo-reorder-test-{}-{}-{n}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn write(&self, rel: &str, contents: &str) {
        let p = self.dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, contents).unwrap();
    }

    fn manifest(&self) -> PathBuf {
        self.dir.join("Cargo.toml")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn canonical(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[test]
fn walks_lib_module_tree() {
    let ws = Workspace::new("lib_tree");
    ws.write(
        "Cargo.toml",
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"
[lib]
path = "src/lib.rs"
"#,
    );
    ws.write("src/lib.rs", "pub mod a;\npub mod b;\n");
    ws.write("src/a.rs", "pub fn a() {}\n");
    ws.write("src/b/mod.rs", "pub mod c;\n");
    ws.write("src/b/c.rs", "pub fn c() {}\n");
    // Orphan file — never referenced via `mod`. Must NOT be discovered.
    ws.write("src/orphan.rs", "pub fn orphan() {}\n");
    // Build artefact under target/. Must NOT be discovered.
    ws.write(
        "target/debug/build/demo/out/generated.rs",
        "pub fn x() {}\n",
    );

    let opts = DiscoverOptions {
        all_packages: false,
        packages: &[],
        manifest_path: Some(&ws.manifest()),
    };
    let found = discover(opts).expect("discover");
    let found: Vec<PathBuf> = found.into_iter().map(|p| canonical(&p)).collect();

    let must = ["src/lib.rs", "src/a.rs", "src/b/mod.rs", "src/b/c.rs"];
    for rel in must {
        let want = canonical(&ws.dir.join(rel));
        assert!(found.contains(&want), "missing {rel} in {found:#?}");
    }

    let must_not = ["src/orphan.rs", "target/debug/build/demo/out/generated.rs"];
    for rel in must_not {
        let unwanted = canonical(&ws.dir.join(rel));
        assert!(!found.contains(&unwanted), "should not discover {rel}");
    }
}

#[test]
fn follows_path_attribute() {
    let ws = Workspace::new("path_attr");
    ws.write(
        "Cargo.toml",
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"
[lib]
path = "src/lib.rs"
"#,
    );
    ws.write(
        "src/lib.rs",
        "#[path = \"weird/place.rs\"]\npub mod relocated;\n",
    );
    ws.write("src/weird/place.rs", "pub fn p() {}\n");

    let opts = DiscoverOptions {
        all_packages: false,
        packages: &[],
        manifest_path: Some(&ws.manifest()),
    };
    let found = discover(opts).expect("discover");
    let found: Vec<PathBuf> = found.into_iter().map(|p| canonical(&p)).collect();
    let want = canonical(&ws.dir.join("src/weird/place.rs"));
    assert!(
        found.contains(&want),
        "missing path-attr file in {found:#?}"
    );
}

#[test]
fn descends_inline_modules() {
    let ws = Workspace::new("inline");
    ws.write(
        "Cargo.toml",
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"
[lib]
path = "src/lib.rs"
"#,
    );
    // Inline `mod outer` containing a `mod inner;` which lives at
    // src/outer/inner.rs.
    ws.write("src/lib.rs", "pub mod outer { pub mod inner; }\n");
    ws.write("src/outer/inner.rs", "pub fn i() {}\n");

    let opts = DiscoverOptions {
        all_packages: false,
        packages: &[],
        manifest_path: Some(&ws.manifest()),
    };
    let found = discover(opts).expect("discover");
    let found: Vec<PathBuf> = found.into_iter().map(|p| canonical(&p)).collect();
    let want = canonical(&ws.dir.join("src/outer/inner.rs"));
    assert!(found.contains(&want), "missing inner.rs in {found:#?}");
}
fn make_workspace_with_two_members() -> Workspace {
    let ws = Workspace::new("ws_two");
    ws.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"alpha\", \"beta\"]\nresolver = \"2\"\n",
    );
    ws.write(
        "alpha/Cargo.toml",
        "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
    );
    ws.write("alpha/src/lib.rs", "pub fn a() {}\n");
    ws.write(
        "beta/Cargo.toml",
        "[package]\nname = \"beta\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n",
    );
    ws.write("beta/src/lib.rs", "pub fn b() {}\n");
    ws
}

#[test]
fn includes_bin_test_example_targets() {
    let ws = Workspace::new("targets");
    ws.write(
        "Cargo.toml",
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "demo"
path = "src/main.rs"

[[test]]
name = "it"
path = "tests/it.rs"

[[example]]
name = "ex"
path = "examples/ex.rs"
"#,
    );
    ws.write("src/main.rs", "fn main() {}\n");
    ws.write("tests/it.rs", "#[test] fn t() {}\n");
    ws.write("examples/ex.rs", "fn main() {}\n");

    let opts = DiscoverOptions {
        all_packages: false,
        packages: &[],
        manifest_path: Some(&ws.manifest()),
    };
    let found = discover(opts).expect("discover");
    let found: Vec<PathBuf> = found.into_iter().map(|p| canonical(&p)).collect();
    for rel in ["src/main.rs", "tests/it.rs", "examples/ex.rs"] {
        let want = canonical(&ws.dir.join(rel));
        assert!(found.contains(&want), "missing {rel} in {found:#?}");
    }
}

#[test]
fn package_flag_filters_to_named_member() {
    let ws = make_workspace_with_two_members();
    let pkgs = ["alpha".to_string()];
    let opts = DiscoverOptions {
        all_packages: false,
        packages: &pkgs,
        manifest_path: Some(&ws.manifest()),
    };
    let found = discover(opts).unwrap();
    let found: Vec<PathBuf> = found.into_iter().map(|p| canonical(&p)).collect();
    let alpha = canonical(&ws.dir.join("alpha/src/lib.rs"));
    let beta = canonical(&ws.dir.join("beta/src/lib.rs"));
    assert!(found.contains(&alpha));
    assert!(!found.contains(&beta));
}

#[test]
fn package_flag_unknown_name_is_an_error() {
    let ws = make_workspace_with_two_members();
    let pkgs = ["does-not-exist".to_string()];
    let opts = DiscoverOptions {
        all_packages: false,
        packages: &pkgs,
        manifest_path: Some(&ws.manifest()),
    };
    let err = discover(opts).unwrap_err();
    assert!(format!("{err}").contains("does-not-exist"), "{err}");
}

#[test]
fn virtual_workspace_default_includes_all_members() {
    let ws = make_workspace_with_two_members();
    let opts = DiscoverOptions {
        all_packages: false,
        packages: &[],
        manifest_path: Some(&ws.manifest()),
    };
    let found = discover(opts).unwrap();
    let found: Vec<PathBuf> = found.into_iter().map(|p| canonical(&p)).collect();
    // No root in a virtual workspace, so default behaviour falls through to
    // every member.
    for rel in ["alpha/src/lib.rs", "beta/src/lib.rs"] {
        let want = canonical(&ws.dir.join(rel));
        assert!(found.contains(&want), "missing {rel} in {found:#?}");
    }
}
