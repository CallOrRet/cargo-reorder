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
//! * `field_sorting/*` — struct/enum field reordering at various sizes.
//! * `single_line_fields/*` — single-line struct literal reordering.
//! * `impl_body/*` — impl/trait body method reordering.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
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

// ── field_sorting: multi-line named-field reordering ──────────────
// Stresses `split_lines` allocations in `sort_top_level_with_options`
// and `byte_offset_for_line_col` for field span → byte-range conversion.
fn field_sorting(c: &mut Criterion) {
    let mut g = c.benchmark_group("field_sorting");
    let cases: &[(&str, String)] = &[
        ("10structs_10fields", field_sorting_source(10, 10)),
        ("10structs_50fields", field_sorting_source(10, 50)),
        ("50structs_10fields", field_sorting_source(50, 10)),
        ("1struct_200fields", field_sorting_source(1, 200)),
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

/// `n_structs` each with `n_fields` named fields (randomised order so
/// sorting actually permutes them). Each field gets a doc-comment line so
/// the field block is multi-line — this forces the line-based sort path.
fn field_sorting_source(n_structs: usize, n_fields: usize) -> String {
    let mut s = String::with_capacity(n_structs * n_fields * 64);
    for si in 0..n_structs {
        let mut fields: Vec<String> = (0..n_fields).map(|fi| format!("field_{fi}")).collect();
        // Deterministic shuffle so sorting has work to do.
        let mut state = DefaultHasher::new();
        (si, n_fields).hash(&mut state);
        let seed = state.finish();
        for i in 0..fields.len() {
            let j = (seed as usize + i * 7 + 13) % fields.len();
            fields.swap(i, j);
        }
        s.push_str(&format!("pub struct S{si} {{\n"));
        for name in &fields {
            let ty = format!("Type{}_{}", si, name);
            s.push_str(&format!("    /// Doc for {name}\n"));
            s.push_str(&format!("    pub {name}: {ty},\n"));
        }
        s.push_str("}\n\n");
    }
    s
}

// ── single_line_fields: single-line struct-literal reordering ──────
// Stresses `byte_range_for_span` → `byte_offset_for_line_col` →
// `split_lines` — called once per field in every single-line list.
fn single_line_fields(c: &mut Criterion) {
    let mut g = c.benchmark_group("single_line_fields");
    let cases: &[(&str, String)] = &[
        ("50funcs_20fields", single_line_source(50, 20)),
        ("200funcs_10fields", single_line_source(200, 10)),
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

/// `n_funcs` functions, each constructing a single-line struct literal
/// with `n_fields` named fields in reversed order (so sorting always
/// permutes). Stresses the single-line reelist byte-range path.
fn single_line_source(n_funcs: usize, n_fields: usize) -> String {
    let mut s = String::with_capacity(n_funcs * n_fields * 48);
    s.push_str("pub struct Data {\n");
    for fi in 0..n_fields {
        s.push_str(&format!("    pub field_{fi}: u32,\n"));
    }
    s.push_str("}\n\n");
    for fi in 0..n_funcs {
        // Write fields from high to low so sorting actually reorders.
        let mut parts: Vec<String> = (0..n_fields)
            .rev()
            .map(|i| format!("field_{i}: {}", i + fi))
            .collect();
        // Deterministic shuffle.
        let mut state = DefaultHasher::new();
        (fi, n_fields).hash(&mut state);
        let seed = state.finish();
        for i in 0..parts.len() {
            let j = (seed as usize + i * 7 + 13) % parts.len();
            parts.swap(i, j);
        }
        let fields_str = parts.join(", ");
        s.push_str(&format!(
            "fn make_{fi}() -> Data {{ Data {{ {fields_str} }} }}\n"
        ));
    }
    s
}

// ── impl_body: impl/trait body method reordering ───────────────────
// Stresses `sort_top_level_with_options` prefix-group hashing and
// the `prefix_of().to_string()` allocation in the inner grouping loop.
fn impl_body(c: &mut Criterion) {
    let mut g = c.benchmark_group("impl_body");
    let cases: &[(&str, String)] = &[
        ("50methods", impl_body_source(50)),
        ("200methods", impl_body_source(200)),
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

/// A struct with an impl block containing `n_methods` methods whose
/// names are deterministically shuffled so the prefix-group sort has
/// real work to do. Mixes sync fn and async fn to hit both buckets.
fn impl_body_source(n_methods: usize) -> String {
    let mut s = String::with_capacity(n_methods * 64);
    s.push_str("pub struct Service;\n\nimpl Service {\n");
    let mut names: Vec<String> = (0..n_methods)
        .map(|i| {
            let prefix = match i % 5 {
                0 => "get",
                1 => "set",
                2 => "build",
                3 => "handle",
                _ => "validate",
            };
            format!("{prefix}_{i}")
        })
        .collect();
    // Deterministic shuffle.
    let mut state = DefaultHasher::new();
    n_methods.hash(&mut state);
    let seed = state.finish();
    for i in 0..names.len() {
        let j = (seed as usize + i * 11 + 7) % names.len();
        names.swap(i, j);
    }
    for (i, name) in names.iter().enumerate() {
        if i % 7 == 0 {
            s.push_str(&format!("    async fn {name}(&self) {{}}\n"));
        } else {
            s.push_str(&format!("    fn {name}(&self) {{}}\n"));
        }
    }
    s.push_str("}\n");
    s
}

// ── enum_sorting: enum with many struct-like variants ─────────────
// Stresses `rewrite_enum`'s repeated `split_lines` calls per variant
// (`field_block_line_range`, `lines_slice`, `byte_range_for_line_range`).
fn enum_sorting(c: &mut Criterion) {
    let mut g = c.benchmark_group("enum_sorting");
    let cases: &[(&str, String)] = &[
        ("20variants_10fields", enum_sorting_source(20, 10)),
        ("50variants_10fields", enum_sorting_source(50, 10)),
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

/// An enum with `n_variants` struct-like variants, each having
/// `n_fields` named fields in deterministically shuffled order.
fn enum_sorting_source(n_variants: usize, n_fields: usize) -> String {
    let mut s = String::with_capacity(n_variants * n_fields * 64);
    s.push_str("#[derive(Debug)]\npub enum LargeEnum {\n");
    for vi in 0..n_variants {
        let mut fields: Vec<String> = (0..n_fields).map(|fi| format!("field_{fi}")).collect();
        let mut state = DefaultHasher::new();
        (vi, n_fields).hash(&mut state);
        let seed = state.finish();
        for i in 0..fields.len() {
            let j = (seed as usize + i * 7 + 13) % fields.len();
            fields.swap(i, j);
        }
        s.push_str(&format!("    Variant{vi} {{\n"));
        for name in &fields {
            let ty = format!("T{vi}_{name}");
            s.push_str(&format!("        /// Doc for {name}\n"));
            s.push_str(&format!("        pub {name}: {ty},\n"));
        }
        s.push_str("    },\n");
    }
    s.push_str("}\n");
    s
}

criterion_group!(
    name = benches;
    config = profiled();
    targets = end_to_end, macro_heavy, imports_heavy, impls_heavy, idempotent, self_file,
             field_sorting, single_line_fields, impl_body, enum_sorting
);
criterion_main!(benches);
