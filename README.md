# cargo-reorder

> English · [中文](./README.zh-CN.md)

Reorder top-level items in Rust source files. There is no single "official"
ordering enforced by `rustfmt` / Rust Style Guide; this tool implements one
common convention from the Rust community and exposes flags so you can switch
to the variant your project prefers.

## Default order

1. `extern crate` declarations
2. `use` imports — three groups separated by a blank line:
   - **std**: `std` / `core` / `alloc`
   - **external**: third-party crates
   - **crate-local**: `crate` → `super` → `self` → local-mod (no blank line inside this block; "local-mod" = a `use foo::...` whose first segment matches a `mod foo;` in the same file)
3. `pub use` re-exports (same grouping)
4. `mod` declarations  *(see `--mod-before-use` below for the minority convention of putting `mod` before `use`)*
5. `const` items
6. `static` items
7. type aliases (`type X = ...`)
8. `trait`s — each followed by its `impl Trait for X` blocks where `X` is not declared in this file (so the impl anchors on the trait instead). *(Use `--struct-before-trait` to swap with #9 / #10.)*
9. `enum`s — each followed by its own `impl` blocks
10. `struct`s / `union`s — each followed by its own `impl` blocks
11. unanchored `impl` blocks — neither the target type nor the trait is declared in this file (e.g. an impl in `submod.rs` whose target lives in `lib.rs`)
12. `extern { ... }` foreign blocks
13. `fn`
14. `async fn`
15. macro definitions (`macro_rules!`)
16. `#[cfg(test)] mod tests` — always last

Within each type's `impl` group the order is **inherent → std trait →
crate-local trait → external trait**. Trait classification looks at
three signals:

1. **Path prefix.** `std::*` / `core::*` / `alloc::*` → std;
   `crate::*` / `self::*` / `super::*` → crate-local.
2. **Local `use` imports** (with `as` rename support).
   `use std::fmt::Display;` or `use std::fmt::Display as D;` lets
   `impl Display for Foo` / `impl D for Foo` count as std-trait.
   Symmetric for crate: `use crate::MyTrait as M;` makes
   `impl M for Foo` count as crate-trait.
3. **File-local trait declarations.** A `trait Foo {}` declared at
   the top of this file makes `impl Foo for X` count as crate-trait,
   even with no `use` line.
4. **Rust prelude.** The 34 traits the compiler auto-imports via
   `std::prelude::v1` + `rust_2021` + `rust_2024` (sourced from
   [`library/std/src/prelude/mod.rs`](https://github.com/rust-lang/rust/blob/master/library/std/src/prelude/mod.rs))
   — markers (`Send`/`Sync`/`Sized`/`Unpin`/`Copy`), the `Fn` and
   `AsyncFn` families, `Drop`, `Clone`/`Default`/`ToOwned`, the four
   `cmp` traits, `From`/`Into`/`TryFrom`/`TryInto`/`AsRef`/`AsMut`,
   the iterator family (`Iterator`/`IntoIterator`/`FromIterator`/
   `Extend`/`DoubleEndedIterator`/`ExactSizeIterator`), `ToString`,
   and (2024) `Future`/`IntoFuture`. These classify as std even
   without an explicit `use`. Non-prelude std traits like `Display`,
   `Debug`, `Read`, `Write`, `Add` still need an import — Rust
   itself requires it, so any working file already has it.

Single-segment names that aren't path-qualified, aren't imported,
aren't declared locally, and aren't in the prelude fall through to
"external".

Within every other category the original relative order is preserved
(stable sort).

A blank line is inserted at every import boundary — between two import
buckets in different (category, visual group) tuples, and between the
last import block and the first non-import item. Spacing among
non-import items is left alone (the user's blank lines are preserved
through the reorder).

## Default order — worked example

A messy file:

```rust
//! crate docs

#![allow(dead_code)]

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}

async fn fetch() {}

fn helper() -> i32 { 1 }

extern "C" {
    fn external();
}

impl other_crate::Trait for Foo { fn ot(&self) {} }

impl std::fmt::Display for Foo {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }
}

impl Foo {
    pub fn new() -> Self { Foo }
}

trait Greet {}

impl Greet for u32 {}

pub struct Foo;

pub enum Color { Red, Green }

type Map = std::collections::HashMap<String, String>;

static COUNTER: u32 = 0;

const MAX: u32 = 100;

mod helpers;
pub mod public_api;

pub use crate::public_api::Reexported;
pub use std::sync::Arc;

use helpers::Helper;
use self::inner::Inner;
use crate::module::Item;
use serde::Serialize;
use std::collections::HashMap;

extern crate alloc;
```

After running `cargo-reorder` (with default settings, output captured
verbatim by piping the input above through the binary):

```rust
//! crate docs

#![allow(dead_code)]

extern crate alloc;

use std::collections::HashMap;

use serde::Serialize;

use crate::module::Item;
use self::inner::Inner;
use helpers::Helper;

pub use std::sync::Arc;

pub use crate::public_api::Reexported;

mod helpers;
pub mod public_api;

const MAX: u32 = 100;

static COUNTER: u32 = 0;

type Map = std::collections::HashMap<String, String>;

trait Greet {}

impl Greet for u32 {}

pub enum Color { Red, Green }

pub struct Foo;

impl Foo {
    pub fn new() -> Self { Foo }
}

impl std::fmt::Display for Foo {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }
}

impl other_crate::Trait for Foo { fn ot(&self) {} }

extern "C" {
    fn external();
}

fn helper() -> i32 { 1 }

async fn fetch() {}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}
```

Things to notice:

* `//! crate docs` and `#![allow(dead_code)]` stay at the top — file-level
  trivia is never reshuffled.
* `extern crate alloc;` lands first, separated by a blank line from
  the `use` block.
* `use` is split into three visually distinct groups (std / external /
  crate-local) with blank lines between them. Inside the third group
  the order is `crate` → `super` → `self` → local-mod (`helpers`,
  matched by the file-level `mod helpers;` declaration).
* `pub use` mirrors the same sub-grouping but sits in its own block
  after `use`.
* No `macro_rules!` in this example. When present, every macro item
  is a barrier (see "Macro items are hard barriers" below) and the
  surrounding sort is split into independent before / after segments.
* `struct Foo` is followed by its three `impl` blocks in the order
  inherent (`impl Foo`) → std trait (`impl Display for Foo`) →
  external trait (`impl other_crate::Trait for Foo`).
* `trait Greet` carries `impl Greet for u32` along — the impl is
  anchored on the trait because the target type is non-local.
* `#[cfg(test)] mod tests` is forced to the end regardless of where it
  appeared in the input.

## CLI flags

Grouped by function.

### Discovery scope (which files to process)

| Flag | Effect |
| --- | --- |
| `--all` | process every workspace member (matches `cargo fmt --all`) |
| `-p`, `--package <NAME>` | process only the named package(s) (matches `cargo fmt -p NAME`); repeat for multiple |
| `--manifest-path <PATH>` | use a specific `Cargo.toml` for discovery |
| `--exclude <substr>` | skip files whose path contains the given substring |

### Item ordering policy

| Flag | Effect |
| --- | --- |
| `--pub-mod-first` | sort `pub mod` before `mod` and add a blank line between the groups (see "On `--pub-mod-first`" below) |
| `--mod-before-use` | put `mod` before `use` (minority convention; see "On `--mod-before-use`" below) |
| `--struct-before-trait` | sort `enum` / `struct` before `trait` (default is trait-first; see "On `--struct-before-trait`" below) |
| `--no-import-groups` | don't split imports into std / external / crate |
| `--no-impl-grouping` | don't anchor `impl` blocks to their type — all impls go in one bucket |
| `--no-tests-last` | don't force `#[cfg(test)] mod tests` to the end |
| `--no-reorder-inline-mods` | don't reorder bodies of inline `mod foo { ... }` blocks (see "On `--no-reorder-inline-mods`" below) |

### Output mode

| Flag | Effect |
| --- | --- |
| `--check` | exit 1 if any file would change (CI mode) |
| `--stdout` | print to stdout instead of rewriting in place |
| `-v`, `--verbose` | log every rewritten file (default mode is silent) |
| `--color <auto\|always\|never>` | colour `--check` diffs (red `-`, green `+`, cyan header) and parse errors. Default `auto`: only when stderr is a tty and `NO_COLOR` is unset. |

> The numbers below come from running `sample-stats`
> (a small `syn`-based binary in this repo, `src/bin/sample-stats.rs`)
> on fresh clones of each project. Using `syn` instead of `grep`
> means all visibility variants (`pub`, `pub(crate)`, `pub(super)`,
> `pub(in path)`) and inline `mod foo { ... }` nesting are handled
> correctly. Each scope (file body OR inline non-test `mod` body)
> contributes one observation; **test mods** (named `tests` /
> `test`, or `#[cfg(test)]`-attributed) are skipped both as their
> own observation and as something to recurse into.

### Reproducing the numbers

`sample-stats` ships as a second binary in this crate. Build it
once and point it at any number of project roots:

```shell
cargo build --release --bin sample-stats

# One project:
./target/release/sample-stats ~/src/serde

# Many at once — totals are aggregated across all of them:
./target/release/sample-stats \
    ~/src/anyhow ~/src/serde ~/src/clap ~/src/regex \
    ~/src/syn ~/src/ripgrep ~/src/tracing ~/src/tokio ~/src/cargo
```

Per-project progress goes to **stderr**; the three Markdown
tables (mod-vs-use, pub/priv mod arrangement, trait-vs-struct)
go to **stdout**, so you can pipe them straight into a doc:

```shell
./target/release/sample-stats ~/src/* > stats.md
```

Each `.rs` file under every given directory is parsed with
`syn::parse_file` (`target/` and `.git/` are skipped). Files
that fail to parse are counted under `parse-skipped` in the
stderr line and don't contribute observations. There are no
flags — every observation rule (test-mod skipping, inline-mod
recursion, visibility handling) is fixed by the source.

## On `--pub-mod-first`

Counting scopes that contain BOTH a `pub mod foo;` and a private
`mod foo;` declaration (external declarations only, not inline
`mod foo { ... }` blocks) — 64 scopes across the 9 projects:

| project | pub-first | priv-first | interleaved |
| --- | --: | --: | --: |
| ripgrep | 0 | 0 | 1 |
| serde | 4 | 2 | 0 |
| syn | 1 | 0 | 1 |
| clap | 1 | 4 | 2 |
| regex | 2 | 1 | 5 |
| tracing | 1 | 3 | 3 |
| cargo | 2 | 2 | 10 |
| tokio | 3 | 2 | 14 |

Aggregate: **22% pub-first, 22% private-first, 56% interleaved**.
Default keeps the relative order from the source — `pub mod` and
`mod` aren't reshuffled by visibility.

## On `--mod-before-use`

There is **no official rule** about whether `mod` or `use` comes
first. Counting scopes that contain both an external `mod foo;`
declaration and a `use ...;` — 254 scopes across the 9 projects
(inline `mod foo { ... }` blocks don't count as `mod` declarations
for this comparison; their bodies are sampled separately as their
own scopes):

| project | use-first | mod-first |
| --- | --: | --: |
| ripgrep | 100% | 0% |
| cargo | 100% | 0% |
| regex | 96% | 4% |
| tracing | 96% | 4% |
| clap | 95% | 5% |
| anyhow | 83% | 17% |
| serde | 78% | 22% |
| tokio | 42% | 58% |
| syn | 15% | 85% |

Aggregate: **76% use-first, 24% mod-first**. Default is use-first;
pass `--mod-before-use` if your project leans the other way (notably
`syn`, which is overwhelmingly mod-first inside its many inline
sub-modules, and `tokio`, which is mod-first across its workspace).

## On `--struct-before-trait`

Counting `trait` vs `struct` / `enum` / `union` declarations in
scopes that have both — 105 scopes across the 9 projects:

| project | trait-first | struct-first |
| --- | --: | --: |
| anyhow | 5 | 0 |
| clap | 9 | 0 |
| ripgrep | 4 | 0 |
| syn | 7 | 0 |
| cargo | 20 | 0 |
| regex | 13 | 1 |
| tracing | 17 | 2 |
| serde | 4 | 4 |
| tokio | 12 | 7 |

Aggregate: **87% trait-first, 13% struct-first** — strongly
trait-first. Most crate-internal abstractions are written as
`pub(crate) trait` *before* the `struct`s that implement them.
Default is trait-first; pass `--struct-before-trait` only if your
project bucks this pattern.

## On `--no-reorder-inline-mods`

By default cargo-reorder recurses into inline `mod foo { ... }`
blocks and applies the same rules to their bodies (and to any
inline mods nested inside). Pass `--no-reorder-inline-mods` to
restrict reordering to the **file top level** only and leave every
inline mod body byte-for-byte untouched.

Recursion deliberately **skips** three patterns where the
listing order is part of the contract or affects compilation:

| Pattern | Why we skip |
| --- | --- |
| `#[cfg(test)] mod ...` / `mod tests { ... }` | already pulled to file end by `--tests-last`; reordering test fixtures hides intent |
| `#[macro_use] mod ...` | `macro_rules!` defined inside leak to the parent scope; reordering inside changes visibility order |
| Pure-`use` mods (every item is `use ...`) | covers `prelude`, `__private`, sealed-trait re-export shims — listing order is the public contract |

A mod with at least one non-`use` item is eligible. Inside an
eligible body, every `macro_rules!` is treated as a barrier just
like at the file top level — it pins in place and forbids body
items from reordering across it.

The flag is off by default because `prelude`-style modules and
codegen scaffolding are common enough that surprise reorders
inside them are worse than the win on "regular" inline mods.
Enable it project-wide once you've eyeballed the diff on a
representative file.

## Comments and attributes

Comments and attributes are preserved:

- doc comments (`///`, `//!`) and `#[...]` attributes stay attached to their item
- a `//` or `/* */` comment immediately above an item moves with that item
- a comment block separated from the item below by a blank line is treated as
  trailing trivia of the item above — or, before the first item, as a
  file-level header that stays at the top
- a `//` comment block surrounded by blank lines on **both** sides is a
  *floating fence*: it stays anchored to its source position and items above
  it are forbidden from reordering past items below it (and vice versa). Use
  this to keep hand-written section dividers intact, e.g.
  ```rust
  // === public API ===

  pub fn ...

  // === helpers ===

  fn ...
  ```
  Each section is sorted independently; the dividers don't move. Doc comments
  (`///`, `//!`) are excluded from this rule because syn associates them with
  the next item as attributes.
- shebang lines and crate-level inner attributes (`#![...]`) always remain at
  the top of the file

## Usage

```shell
# Rewrite files in place
cargo-reorder src/

# Rewrite a single file
cargo-reorder src/lib.rs

# Print to stdout instead of editing
cargo-reorder --stdout src/lib.rs

# Read from stdin / write to stdout
cargo-reorder < input.rs > output.rs

# CI mode: exit 1 if anything would change
cargo-reorder --check src/
```

## Tested

The tool has been validated end-to-end against several large real-world
crates. For each one we cloned a fresh copy, ran `cargo check` and
`cargo test` to record a baseline, then reordered every file under the
crate, re-ran `cargo check` to confirm the project still compiles, and
re-ran the tests to confirm test results are identical to the baseline.

| project | total `.rs` | LOC | files reordered | reorder time | compiles | tests |
| --- | --: | --: | --: | --: | :-: | :-: |
| [anyhow](https://github.com/dtolnay/anyhow) | 37 | 5,833 | 21 | 0.11 s | ✅ | ✅ (3) |
| [serde](https://github.com/serde-rs/serde) | 208 | 42,630 | 42 | 0.23 s | ✅ | ✅ (5) |
| [ripgrep](https://github.com/BurntSushi/ripgrep) | 100 | 52,266 | 42 | 0.46 s | ✅ | * (no `[lib]` target) |
| [syn](https://github.com/dtolnay/syn) | 133 | 68,988 | 71 | 0.59 s | ✅ ** | — (suite needs `rustc-dev`, nightly) |
| [tracing](https://github.com/tokio-rs/tracing) | 260 | 71,547 | 123 | 0.51 s | ✅ | ✅ (188) |
| [clap](https://github.com/clap-rs/clap) | 329 | 83,179 | 80 | 0.47 s | ✅ | ✅ |
| [regex](https://github.com/rust-lang/regex) | 225 | 159,330 | 83 | 1.38 s | ✅ | ✅ (7) |
| [tokio](https://github.com/tokio-rs/tokio) | 777 | 173,843 | 254 | 0.65 s | ✅ | ✅ (lib, 146) |
| [cargo](https://github.com/rust-lang/cargo) | 1,352 | 333,542 | 782 | 2.42 s | ✅ | ✅ (lib, 160) |

`*` ripgrep is a binary crate with no `[lib]` target; `cargo test
--lib` is N/A, but `cargo check --all-targets` passes after reorder.
`**` syn's compile result is byte-identical to baseline — its test
suite gates on `--all-features`, which pulls in `rustc-dev` internal
crates only available on nightly. Not caused by our reorder.

Numbers: `cargo-reorder` running release-built and single-threaded
against a fresh clone of each project. "files reordered" counts
files whose contents actually changed; the rest were already in
canonical order.

The reorderer is also idempotent on every project: a second `--check`
pass after the first reorder reports no further changes.

### Test suite

The unit + integration suite is organised by topic, one file per
concern:

| file | what it covers |
| --- | --- |
| `tests/items.rs` | each of the 16 top-level item categories lands in its slot |
| `tests/imports.rs` | `extern crate` / `use` / `pub use` grouping and renames |
| `tests/impls.rs` | impl anchoring and the 4-tier inherent → std → crate → external order |
| `tests/macros.rs` | macro-as-barrier semantics: pinning, segment isolation, idempotence |
| `tests/cross_file.rs` | `mod foo;` lookup, `#[path]` overrides, missing-child fallback |
| `tests/comments.rs` | leading comments, inner doc, file-level header blocks |
| `tests/attributes.rs` | `#[derive(...)]` / `#[cfg(...)]` / `#[cfg_attr]` / multi-line attrs |
| `tests/generics.rs` | lifetimes, where clauses, const generics, GAT, HRTB, `async fn in trait` |
| `tests/visibility.rs` | `pub` / `pub(crate)` / `pub(super)` / `pub(in path)` round-trip |
| `tests/flags.rs` | every `Config` flag end-to-end |
| `tests/frontmatter.rs` | RFC 3502 cargo-script frontmatter |
| `tests/idempotence.rs` | round-trip stability on representative files |
| `tests/edge_cases.rs` | unicode idents, raw strings, many blank lines, inline mods, EOF without `\n` |
| `tests/discover.rs` | `cargo metadata` based file discovery, `--all` / `-p` / `--manifest-path` |

### Caveats discovered during validation

* **Macro items are hard barriers.** `macro_rules!` is textually
  scoped, and call sites can hide in many shapes (struct-field-init
  position, type annotations, deeply nested in a sibling file
  reached through `#[macro_use] mod`, …). Rather than try to detect
  every caller correctly, the reorderer pins every macro-related
  top-level item in its source position and forbids any other item
  from reordering across it. The barrier set:
  - `macro_rules! foo` (with or without ident) — pins itself
  - bare top-level `lazy_static! { ... }` / `to_hash_map!(...)` style
    invocations — pin themselves
  - `#[macro_use] mod foo;` — pins itself, since its child's exported
    macros leak into this scope from this point downward

  Trade-off: a file with several macros gets split into multiple
  independent sort segments, so the result is sometimes less
  thoroughly reordered than it could be. In exchange, the rule is
  trivial to reason about and never produces uncompilable output.
* **RFC 3502 cargo-script files are supported.** Single-file scripts
  with a leading `---` ... `---` TOML frontmatter (with or without a
  preceding shebang) are detected, the frontmatter is preserved
  verbatim, and only the Rust body is reordered. This works for both
  the bare-`---` opener and the `---cargo` info-string variant.
* **Parse errors are strict on build files, silent off-tree.**
  Mirroring `cargo fmt`, files reachable from a target's `src_path`
  through the module tree (i.e. files that the project actually
  compiles) are treated strictly: a parse error on them is reported
  as a rustc-style diagnostic and exits non-zero. Files **not** in
  the build tree (notably cargo's own
  `tests/testsuite/script/rustc_fixtures/`, rustfix's
  `tests/everything/`, and other test fixtures with `.rs` extension
  that aren't compiled) are silently skipped on parse failure. This
  classification uses the same `cargo metadata` output that drives
  default discovery, so it works whether you run `cargo-reorder`
  with no arguments, with `--all`, or with explicit paths inside a
  cargo project.

## License

Apache-2.0
