//! `--fmt` delegates to `cargo fmt`. CLI shape mirrors cargo fmt's,
//! so whatever cargo fmt would format with the same `--all` / `-p` /
//! `--manifest-path` selection, cargo-reorder operates on the same
//! files. Skipped under `--check` so a CI gate doesn't write to disk.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static UNIQ: AtomicU64 = AtomicU64::new(0);

fn binary_path() -> PathBuf {
    env::var_os("CARGO_BIN_EXE_cargo-reorder")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut p = env::current_exe().unwrap();
            p.pop();
            p.pop();
            p.push("cargo-reorder");
            p
        })
}

fn cargo_fmt_present() -> bool {
    Command::new("cargo")
        .arg("fmt")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct TempDir {
    dir: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let mut dir = env::temp_dir();
        let n = UNIQ.fetch_add(1, Ordering::Relaxed);
        dir.push(format!(
            "cargo-reorder-fmt-{name}-{}-{n}",
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

#[test]
fn fmt_flag_runs_cargo_fmt_then_reorders() {
    if !cargo_fmt_present() {
        eprintln!("skipping: cargo fmt not available");
        return;
    }
    let td = TempDir::new("metadata");
    td.write(
        "Cargo.toml",
        "[package]
name = \"fmt_pkg\"
version = \"0.0.1\"
edition = \"2024\"
",
    );
    // Badly-spaced source; cargo fmt should clean it up before our
    // reorder pass sees it. Reorder will then put `cache_get` ahead
    // of `user_login` by the mean-length rule.
    td.write(
        "src/lib.rs",
        "fn  user_login(  ) {  }
fn  cache_get(){}
",
    );
    let status = Command::new(binary_path())
        .arg("--fmt")
        .arg("-p")
        .arg("fmt_pkg")
        .current_dir(&td.dir)
        .status()
        .unwrap();
    assert!(status.success(), "exit: {status}");
    let after = fs::read_to_string(td.dir.join("src/lib.rs")).unwrap();
    let p_cache = after
        .find("fn cache_get() {}")
        .expect("cargo fmt should have cleaned spacing");
    let p_user = after
        .find("fn user_login() {}")
        .expect("cargo fmt should have cleaned spacing");
    assert!(p_cache < p_user, "reorder should run after fmt:\n{after}");
}

#[test]
fn rustfmt_options_after_double_dash_are_forwarded() {
    // Pass an unknown rustfmt flag through `cargo-reorder --fmt --
    // --bogus-rustfmt-flag` and confirm cargo fmt sees it (and fails).
    // If we weren't forwarding, cargo fmt would succeed on the
    // well-formed source and the test would pass falsely.
    if !cargo_fmt_present() {
        return;
    }
    let td = TempDir::new("passthrough");
    td.write(
        "Cargo.toml",
        "[package]
name = \"pass_pkg\"
version = \"0.0.1\"
edition = \"2024\"
",
    );
    td.write("src/lib.rs", "fn a() {}\nfn b() {}\n");
    let status = Command::new(binary_path())
        .arg("--fmt")
        .arg("-p")
        .arg("pass_pkg")
        .arg("--")
        .arg("--bogus-rustfmt-flag-xyz")
        .current_dir(&td.dir)
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "bogus rustfmt flag should propagate through cargo fmt and fail"
    );
}

#[test]
fn fmt_flag_skipped_under_check_mode() {
    if !cargo_fmt_present() {
        return;
    }
    let td = TempDir::new("check");
    td.write(
        "Cargo.toml",
        "[package]
name = \"check_pkg\"
version = \"0.0.1\"
edition = \"2024\"
",
    );
    let original = "fn  user_a(  ) {}
fn  user_b(){}
";
    td.write("src/lib.rs", original);
    let _ = Command::new(binary_path())
        .arg("--fmt")
        .arg("--check")
        .arg("-p")
        .arg("check_pkg")
        .current_dir(&td.dir)
        .output()
        .unwrap();
    // `--check` is read-only; neither cargo fmt (we skip it) nor the
    // reorder pass (it only writes when `--check` is off) should
    // touch the file.
    let after = fs::read_to_string(td.dir.join("src/lib.rs")).unwrap();
    assert_eq!(after, original, "file must not be touched under --check");
}
