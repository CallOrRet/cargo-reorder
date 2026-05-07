//! Filter mode: with no selection flag and a piped stdin, cargo-reorder
//! reads source from stdin and writes the reordered result to stdout.

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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

#[test]
fn filter_mode_parse_error_exits_2() {
    let input = "fn broken( {\n";
    let mut child = Command::new(binary_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cargo-reorder");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert_eq!(out.status.code(), Some(2), "parse error → exit 2");
}
#[test]
fn filter_mode_reorders_stdin_to_stdout() {
    // Two free fns; the mean-length rule places `cache_get` ahead of
    // `user_login`. Source is well-formed so no rustfmt is needed.
    let input = "fn user_login() {}\nfn cache_get() {}\n";
    let mut child = Command::new(binary_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cargo-reorder");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "exit: {}", out.status);
    let stdout = String::from_utf8(out.stdout).unwrap();
    let p_cache = stdout.find("fn cache_get").expect("cache_get in stdout");
    let p_user = stdout.find("fn user_login").expect("user_login in stdout");
    assert!(p_cache < p_user, "filter mode should reorder:\n{stdout}");
}

