//! Performance benchmarks for cargo-reorder.
//!
//! Run all:        `cargo bench`
//! Run one group:  `cargo bench -- end_to_end`
//! Filter by name: `cargo bench -- macro_heavy`
//!
//! The synthetic generators below build inputs that stress specific code
//! paths so future regressions show up against named baselines:
//!
//! * `end_to_end/*` — overall pipeline at small/medium/large sizes.
//! * `macro_heavy`  — many `macro_rules!` items as barrier segments.
//! * `imports_heavy`— import classification + std/crate origin tracking.
//! * `impls_heavy`  — `impl` anchor resolution in compute_sort_key.
//! * `idempotent`   — second pass over already-sorted output.
//! * `self_file`    — this project's own `src/reorder.rs` (~840 lines).

use std::fs;
use std::hint::black_box;
use std::path::PathBuf;

use cargo_reorder::{Config, reorder_source_with};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

fn cfg() -> Config {
    Config::default()
}

// Custom criterion config: when invoked with `--profile-time <secs>`,
// criterion calls into the profiler and emits a `pprof` protobuf
// (`target/criterion/<group>/<id>/profile/profile.pb`) which can be
// inspected with `pprof -top profile.pb`, `go tool pprof`, or grepped
// directly for symbol names. 4000 Hz sampling resolves microsecond-level
// per-iter functions; the OS hard cap is around 10 kHz on Linux.
#[cfg(unix)]
fn profiled() -> Criterion {
    Criterion::default().with_profiler(pprof::criterion::PProfProfiler::new(
        4000,
        pprof::criterion::Output::Protobuf,
    ))
}

#[cfg(not(unix))]
fn profiled() -> Criterion {
    Criterion::default()
}

/// Real-world baseline: this project's own `src/reorder.rs`.
fn self_file(c: &mut Criterion) {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let path: PathBuf = [manifest, "src", "reorder.rs"].iter().collect();
    let src = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return, // skip silently if file moved/renamed
    };
    let mut g = c.benchmark_group("self_file");
    g.throughput(Throughput::Bytes(src.len() as u64));
    g.bench_function("src/reorder.rs", |b| {
        b.iter(|| {
            let _ = reorder_source_with(black_box(&src), black_box(&cfg())).unwrap();
        });
    });
    g.finish();
}

fn end_to_end(c: &mut Criterion) {
    let mut g = c.benchmark_group("end_to_end");
    let cases: &[(&str, String)] = &[
        ("small", small_source()),
        ("medium_100", mixed_source(100)),
        ("large_500", mixed_source(500)),
    ];
    for (name, src) in cases {
        g.throughput(Throughput::Bytes(src.len() as u64));
        g.bench_function(*name, |b| {
            b.iter(|| {
                let _ = reorder_source_with(black_box(src), black_box(&cfg())).unwrap();
            });
        });
    }
    g.finish();
}

/// Second pass over already-reordered source. The sort is a no-op,
/// so this isolates the parse + walk + emit cost.
fn idempotent(c: &mut Criterion) {
    let mut g = c.benchmark_group("idempotent");
    let src = mixed_source(200);
    let sorted = reorder_source_with(&src, &cfg()).unwrap();
    g.throughput(Throughput::Bytes(sorted.len() as u64));
    g.bench_function("medium_200", |b| {
        b.iter(|| {
            let _ = reorder_source_with(black_box(&sorted), black_box(&cfg())).unwrap();
        });
    });
    g.finish();
}

/// Tiny file: a handful of items in mixed order.
fn small_source() -> String {
    String::from(
        "\
fn helper() {}

const X: u32 = 1;

struct S;

use std::collections::HashMap;

impl S { fn new() -> Self { S } }

mod sub;

trait T {}
",
    )
}

/// Generate an out-of-order file with `n` mixed items: `use`, `mod`,
/// `const`, `struct`, `trait`, and `impl` blocks anchoring back to
/// earlier types. Designed to make every category bucket non-trivial.
fn mixed_source(n: usize) -> String {
    let mut s = String::with_capacity(n * 64);
    for i in 0..n {
        match i % 6 {
            0 => s.push_str(&format!("fn f_{i}() {{}}\n\n")),
            1 => s.push_str(&format!("use std::collections::HashMap as H_{i};\n\n")),
            2 => s.push_str(&format!("const C_{i}: u32 = {i};\n\n")),
            3 => s.push_str(&format!("struct S_{i};\n\n")),
            4 => s.push_str(&format!("trait T_{i} {{}}\n\n")),
            _ => {
                let target = i.saturating_sub(2);
                s.push_str(&format!("impl S_{target} {{ fn m_{i}(&self) {{}} }}\n\n"));
            }
        }
    }
    s
}

fn macro_heavy(c: &mut Criterion) {
    let mut g = c.benchmark_group("macro_heavy");
    let cases: &[(&str, String)] = &[
        ("10x10", macro_heavy_source(10, 10)),
        ("20x20", macro_heavy_source(20, 20)),
        ("50x10", macro_heavy_source(50, 10)),
    ];
    for (name, src) in cases {
        g.throughput(Throughput::Bytes(src.len() as u64));
        g.bench_function(*name, |b| {
            b.iter(|| {
                let _ = reorder_source_with(black_box(src), black_box(&cfg())).unwrap();
            });
        });
    }
    g.finish();
}

/// `m` macro definitions, each invoked by `c` non-macro callers.
/// Stresses the macro-as-barrier segment computation: each macro
/// punches a private segment that splits its surrounding callers.
fn macro_heavy_source(macros: usize, callers_per_macro: usize) -> String {
    let mut s = String::new();
    // Definitions placed at the END of the file so the segment
    // computation has many caller items to walk before each barrier.
    let mut callers = String::new();
    for mi in 0..macros {
        for ci in 0..callers_per_macro {
            callers.push_str(&format!("fn caller_{mi}_{ci}() {{ m_{mi}!(); }}\n"));
        }
    }
    s.push_str(&callers);
    s.push('\n');
    for mi in 0..macros {
        s.push_str(&format!("macro_rules! m_{mi} {{ () => {{ {mi} }}; }}\n"));
    }
    s
}

fn impls_heavy(c: &mut Criterion) {
    let mut g = c.benchmark_group("impls_heavy");
    let cases: &[(&str, String)] = &[
        ("20types_5each", impls_heavy_source(20, 5)),
        ("50types_5each", impls_heavy_source(50, 5)),
    ];
    for (name, src) in cases {
        g.throughput(Throughput::Bytes(src.len() as u64));
        g.bench_function(*name, |b| {
            b.iter(|| {
                let _ = reorder_source_with(black_box(src), black_box(&cfg())).unwrap();
            });
        });
    }
    g.finish();
}

/// `n_types` types each with `impls_per_type` impls (mix of inherent /
/// std trait / external trait). Stresses `impl` anchor + classify.
fn impls_heavy_source(n_types: usize, impls_per_type: usize) -> String {
    let mut s = String::new();
    s.push_str("use std::fmt::Debug;\n\n");
    // Place impls FIRST so the sort has to move every one of them past
    // its target type definition.
    for ti in 0..n_types {
        for ii in 0..impls_per_type {
            match ii % 3 {
                0 => s.push_str(&format!("impl T_{ti} {{ fn m_{ii}(&self) {{}} }}\n")),
                1 => s.push_str(&format!(
                    "impl Debug for T_{ti} {{ fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ Ok(()) }} }}\n"
                )),
                _ => s.push_str(&format!(
                    "impl serde::Serialize for T_{ti} {{ fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {{ s.serialize_unit() }} }}\n"
                )),
            }
        }
    }
    for ti in 0..n_types {
        s.push_str(&format!("struct T_{ti};\n"));
    }
    s
}

fn imports_heavy(c: &mut Criterion) {
    let mut g = c.benchmark_group("imports_heavy");
    let cases: &[(&str, String)] = &[
        ("100", imports_heavy_source(100)),
        ("500", imports_heavy_source(500)),
    ];
    for (name, src) in cases {
        g.throughput(Throughput::Bytes(src.len() as u64));
        g.bench_function(*name, |b| {
            b.iter(|| {
                let _ = reorder_source_with(black_box(src), black_box(&cfg())).unwrap();
            });
        });
    }
    g.finish();
}

/// `n` `use` items split roughly evenly across std / external / crate /
/// self / super. Hits the import-grouping path.
fn imports_heavy_source(n: usize) -> String {
    let mut s = String::with_capacity(n * 32);
    s.push_str("mod local_a;\nmod local_b;\n\n");
    for i in 0..n {
        match i % 5 {
            0 => s.push_str(&format!("use std::collections::HashMap as H_{i};\n")),
            1 => s.push_str(&format!("use serde::Deserialize as D_{i};\n")),
            2 => s.push_str(&format!("use crate::other::Item_{i};\n")),
            3 => s.push_str(&format!("use self::nested::Item_{i};\n")),
            _ => s.push_str(&format!("use super::sibling::Item_{i};\n")),
        }
    }
    s.push_str("\nstruct Anchor;\n");
    s
}

criterion_group!(
    name = benches;
    config = profiled();
    targets = end_to_end, macro_heavy, imports_heavy, impls_heavy, idempotent, self_file
);
criterion_main!(benches);
