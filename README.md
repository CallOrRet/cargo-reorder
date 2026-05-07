# cargo-reorder

> English · [中文](./README.zh-CN.md)

Reorder top-level items in Rust source files. There is no single "official"
ordering enforced by `rustfmt` / Rust Style Guide; this tool implements one
common convention from the Rust community and exposes flags so you can switch
to the variant your project prefers.

## Default order

1. `extern crate` declarations
2. `mod` declarations  *(see `--no-mod-before-use` below to flip with `use`)*
3. `use` imports — three groups separated by a blank line:
   - **std**: `std` / `core` / `alloc`
   - **external**: third-party crates
   - **crate-local**: `crate` → `super` → `self` → local-mod (no blank line inside this block; "local-mod" = a `use foo::...` whose first segment matches a `mod foo;` in the same file)
4. `pub use` re-exports (same grouping)
5. `const` items
6. `static` items
7. type aliases (`type X = ...`)
8. `trait`s — each followed by its `impl Trait for X` blocks where `X` is not declared in this file (so the impl anchors on the trait instead). *(Use `--no-trait-before-struct` to swap with #9 / #10.)*
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

Within a single classification bucket, **shorter trait paths sort
first** (on by default; disable with `--no-short-trait-path-first`).
So `impl Default for Foo` precedes `impl std::default::Default for Foo`
when both target the same type. With the flag off, source order is
preserved.

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

mod helpers;
pub mod public_api;

use std::collections::HashMap;

use serde::Serialize;

use crate::module::Item;
use self::inner::Inner;
use helpers::Helper;

pub use std::sync::Arc;

pub use crate::public_api::Reexported;

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
  the rest.
* `mod` declarations come next (default is mod-first; pass
  `--no-mod-before-use` to flip).
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

The selection flags are intentionally a subset of `cargo fmt`'s — the
same combination always produces the same file set. Direct file or
directory arguments are not supported (cargo fmt doesn't accept them
either): work at the package level instead.

| Flag | Effect |
| --- | --- |
| `--all` | process every workspace member (matches `cargo fmt --all`) |
| `-p`, `--package <NAME>` | process only the named package(s) (matches `cargo fmt -p NAME`); repeat for multiple |
| `--manifest-path <PATH>` | use a specific `Cargo.toml` for discovery |

### Item ordering policy

| Flag | Effect |
| --- | --- |
| `--no-tests-last` | don't force `#[cfg(test)] mod tests` to the end |
| `--no-impl-grouping` | don't anchor `impl` blocks to their type — all impls go in one bucket |
| `--no-import-groups` | don't split imports into std / external / crate |
| `--no-mod-before-use` | put `use` before `mod` (default is mod-first; see "On `--no-mod-before-use`" below) |
| `--no-reorder-fields` | preserve source order of named fields inside `struct` / `union` / `enum` (default groups them by snake_case / PascalCase prefix and sorts shortest-first within each group, see "On `--no-reorder-fields`" below) |
| `--no-reorder-impl-fns` | preserve source order of members inside every `impl` / `trait` body (covers `const` / `type` / `fn` / `async fn`); the field-level and top-level grouping still apply |
| `--no-preserve-mod-order` | sort `pub mod` before `mod` and add a blank line between the groups (see "On `--no-preserve-mod-order`" below) |
| `--no-reorder-inline-mods` | don't reorder bodies of inline `mod foo { ... }` blocks (see "On `--no-reorder-inline-mods`" below) |
| `--no-trait-before-struct` | sort `enum` / `struct` before `trait` (default is trait-first; see "On `--no-trait-before-struct`" below) |
| `--no-short-trait-path-first` | preserve source order between trait impls with different path lengths (default is shortest-first) |

### Output mode

| Flag | Effect |
| --- | --- |
| `--fmt` | run `cargo fmt` (with the same `--all` / `-p` / `--manifest-path` you passed) before reordering. Skipped under `--check` so a CI gate doesn't write to disk. |
| `--check` | exit 1 if any file would change (CI mode) |
| `-v`, `--verbose` | log every rewritten file (default mode is silent) |
| `--color <auto\|always\|never>` | colour `--check` diffs (red `-`, green `+`, cyan header) and parse errors. Default `auto`: only when stderr is a tty and `NO_COLOR` is unset. |

> The numbers below come from running `sample-stats`
> (a small `syn`-based binary in this repo, `src/bin/sample-stats.rs`)
> on fresh clones of each project. Using `syn` instead of `grep`
> means all visibility variants (`pub`, `pub(crate)`, `pub(super)`,
> `pub(in path)`) and inline `mod foo { ... }` nesting are handled
> correctly. Each scope (file body OR inline non-test `mod` body)
> contributes one observation. For two item kinds A and B, the
> scope's verdict is **pair-majority**: count every (a, b) pair and
> see whether more of them have a-before-b or b-before-a; the
> winning side gets the vote, an exact split is "tied". This is
> robust to zig-zag layouts (`mod a; use x; mod b; use y; ...`)
> in a way that "first occurrence" is not. **Test mods** (named
> `tests` / `test`, or `#[cfg(test)]`-attributed) are skipped
> entirely.

### Reproducing the numbers

`sample-stats` ships as a second binary in this crate. Build it
once and point it at any number of project roots:

```shell
cargo build --release --bin sample-stats

# One project:
./target/release/sample-stats ~/src/serde

# Many at once — each project votes once in the rollup:
./target/release/sample-stats \
    ~/src/anyhow ~/src/axum ~/src/bat ~/src/bevy ~/src/cargo \
    ~/src/chrono ~/src/clap ~/src/diesel ~/src/hyper ~/src/itertools \
    ~/src/rayon ~/src/regex ~/src/reqwest ~/src/ripgrep ~/src/rust-analyzer \
    ~/src/serde ~/src/syn ~/src/thiserror ~/src/tokio ~/src/tower ~/src/tracing
```

Per-project progress goes to **stderr**; the three Markdown
tables (mod-vs-use, pub/priv mod arrangement, trait-vs-struct)
go to **stdout**, so you can pipe them straight into a doc:

```shell
./target/release/sample-stats ~/src/* > stats.md
```

Each `.rs` file under every given directory is parsed with
`syn::parse_file`. Skipped at the directory level: `target/`,
`.git/`, plus the conventional non-library directories
`tests/`, `examples/`, and `benches/` — those follow integration
conventions (`mod common;` shims, derive-UI fixtures, throw-away
structs) that don't reflect how the project's library source
organises items. Files that fail to parse are counted under
`parse-skipped` in the stderr line and don't contribute
observations. There are no flags — every observation rule
(test-mod skipping, inline-mod recursion, visibility handling)
is fixed by the source.

## On `--no-preserve-mod-order`

Counting scopes that contain BOTH a `pub mod foo;` and a private
`mod foo;` declaration (external declarations only, not inline
`mod foo { ... }` blocks) — 200 scopes across 19 of the 21 projects
(thiserror and anyhow have no qualifying scopes):

| project | pub-first | priv-first | tied |
| --- | --: | --: | --: |
| axum | 4 | 5 | 0 |
| bat | 1 | 2 | 0 |
| bevy | 25 | 21 | 1 |
| cargo | 4 | 5 | 5 |
| chrono | 1 | 1 | 1 |
| clap | 2 | 5 | 0 |
| diesel | 17 | 7 | 1 |
| hyper | 2 | 1 | 0 |
| itertools | 1 | 0 | 1 |
| rayon | 2 | 0 | 0 |
| regex | 6 | 1 | 1 |
| reqwest | 3 | 0 | 1 |
| ripgrep | 0 | 1 | 0 |
| rust-analyzer | 14 | 13 | 1 |
| serde | 4 | 2 | 0 |
| syn | 1 | 1 | 0 |
| tokio | 8 | 9 | 2 |
| tower | 7 | 3 | 0 |
| tracing | 4 | 3 | 0 |

Aggregate by project (each project votes once for whichever side
wins its internal majority, ties counted separately): **10/19
pub-first (53%), 5/19 priv-first (26%), 4/19 tied (21%)**.
Slight pub-first lean. Default still keeps the relative order from
the source — turning the flip on by default would silently shuffle
files for the ~26% of projects that lean priv-first plus the ~21%
that are split. Pass `--no-preserve-mod-order` if your project follows the
majority pattern.

## On `--no-mod-before-use`

There is **no official rule** about whether `mod` or `use` comes
first; rustfmt is silent on this. Counting scopes that contain both
an external `mod foo;` declaration and a `use ...;` — 618 scopes
across the 21 projects
(inline `mod foo { ... }` blocks don't count as `mod` declarations
for this comparison; their bodies are sampled separately as their
own scopes):

Counts are scope-vote tallies (each scope votes once via
pair-majority for use-first / mod-first / tied):

| project | use-first | mod-first | tied |
| --- | --: | --: | --: |
| anyhow | 0 | 1 | 0 |
| axum | 8 | 11 | 2 |
| bat | 3 | 2 | 0 |
| bevy | 40 | 133 | 1 |
| cargo | 28 | 13 | 1 |
| chrono | 6 | 3 | 0 |
| clap | 4 | 16 | 0 |
| diesel | 16 | 33 | 2 |
| hyper | 5 | 5 | 0 |
| itertools | 1 | 1 | 0 |
| rayon | 4 | 5 | 2 |
| regex | 21 | 3 | 0 |
| reqwest | 4 | 2 | 0 |
| ripgrep | 11 | 1 | 0 |
| rust-analyzer | 31 | 85 | 1 |
| serde | 2 | 11 | 0 |
| syn | 0 | 2 | 0 |
| thiserror | 0 | 2 | 0 |
| tokio | 9 | 39 | 0 |
| tower | 2 | 22 | 1 |
| tracing | 15 | 7 | 1 |

Aggregate by project (each project votes once for whichever side
wins its internal majority, ties counted separately): **7/21 use-first
(33%), 12/21 mod-first (57%), 2/21 tied (10%)**. The default is
**mod-first** to match this majority. Pass `--no-mod-before-use` if
your project is on the use-first side (notably `regex`, `ripgrep`,
`cargo`, `chrono`, `tracing`, `rayon`, `reqwest`).

## On `--no-trait-before-struct`

Counting `trait` vs `struct` / `enum` / `union` declarations in
scopes that have both — 455 scopes across 20 of the 21 projects
(thiserror has no qualifying scopes):

| project | trait-first | struct-first | tied |
| --- | --: | --: | --: |
| anyhow | 1 | 2 | 1 |
| axum | 7 | 2 | 2 |
| bat | 2 | 1 | 0 |
| bevy | 94 | 47 | 12 |
| cargo | 11 | 7 | 0 |
| chrono | 1 | 1 | 0 |
| clap | 4 | 4 | 1 |
| diesel | 27 | 34 | 8 |
| hyper | 4 | 2 | 3 |
| itertools | 7 | 3 | 2 |
| rayon | 10 | 3 | 0 |
| regex | 7 | 6 | 1 |
| reqwest | 7 | 5 | 0 |
| ripgrep | 2 | 2 | 0 |
| rust-analyzer | 28 | 31 | 3 |
| serde | 3 | 2 | 0 |
| syn | 3 | 2 | 2 |
| tokio | 11 | 4 | 2 |
| tower | 10 | 2 | 0 |
| tracing | 12 | 7 | 0 |

Aggregate by project (each project votes once for whichever side
wins its internal majority, ties counted separately): **14/20
trait-first (70%), 3/20 struct-first (15%), 3/20 tied (15%)**.
Strongly trait-first. Default is trait-first; pass
`--no-trait-before-struct` if your project bucks this pattern.

## On `--no-reorder-fields`

By default cargo-reorder applies the same grouping rule at **three
levels**:

- **Field-level**: named fields inside each `struct` / `union`,
  named fields inside each struct-like enum variant, and the
  variants of each `enum`.
- **Top-level (within category)**: among consecutive top-level
  `struct` / `union` / `enum` / `trait` / `fn` / `async fn` items,
  same-category siblings are reordered by the same prefix-grouping +
  length rules. An `impl` block follows whichever type it anchors
  to, so `struct Foo` + `impl Foo` stays together when `Foo` moves.
- **Inside `impl` and `trait` bodies**: members are sorted into the
  same category order used at the top level — `const` → `type` →
  `fn` → `async fn` — and within each category the prefix-group +
  length rule applies. Macro / verbatim members are barriers: their
  presence leaves the whole body verbatim.

The rules in all three cases:

1. **Group** by the identifier's first "word":
   - For snake_case: text up to the first `_` (`foo_bar` → `foo`).
   - For PascalCase / camelCase: text up to the first
     lowercase→uppercase boundary (`FooBar` → `Foo`,
     `fooBar` → `foo`). Names with no boundary at all
     (`Foo`, `BAR`, `XMLParser`) are their own one-element group.
2. **Within a group**: sort by name length, shortest first
   (`bar_x` before `bar_long_name`). Ties preserve source order.
3. **Between groups**: order ascending by the group's **mean
   name length** (sum of all member names' lengths / member count).
   Ties preserve source order.
4. A **blank line** separates consecutive groups; within a group
   fields stay packed together.

Worked example (input on the left, output on the right):

```rust
struct Foo {                  struct Foo {
    foo_loooong: String,          bar_x: u8,
    bar_x: u8,                    bar_y: u32,
    foo_short: u8,
    foo_medium: bool,             foo_short: u8,
    bar_y: u32,                   foo_medium: bool,
}                                 foo_loooong: String,
                              }
```

Pass `--no-reorder-fields` to disable all three passes — top-level
items fall back to category-then-source-order, the inside of each
`struct` / `union` / `enum` is left exactly as written, and `impl` /
`trait` bodies stay in source order. Pass the more targeted
`--no-reorder-impl-fns` if you only want to keep `impl` / `trait`
bodies in source order while keeping the other two passes on (e.g.
when methods follow a deliberate sequence — builder chains,
lifecycle order — but field grouping is still desirable).

The field-level pass automatically **skips** any item whose layout
or derived semantics would change with reordering:

| Pattern | Why we skip |
| --- | --- |
| Any `#[repr(...)]` (C, packed, transparent, align(N), int reprs) | reordering would change ABI / memory layout |
| `#[derive(Ord)]` or `#[derive(PartialOrd)]` | derived comparison reads fields/variants in source order |
| `enum` with any explicit discriminant (`A = 1`) | reordering silently shifts the implicit values for the rest |
| Tuple struct / tuple variant / unit struct / unit variant | no field names to group on |
| Single-line layouts (`struct S { a: u8, b: u8 }`) | line-based slicing can't disentangle them — left intact |
| Fewer than 2 fields | nothing to sort |

These skips are conservative on purpose: the goal is "field
reorder is always safe to apply with default settings". If your
codebase has a case we should also skip, open an issue.

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
| `#[cfg(test)] mod ...` / `mod tests { ... }` | already pulled to file end by the default tests-last rule (off via `--no-tests-last`); reordering test fixtures hides intent |
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
# Reorder the current package
cargo-reorder

# Reorder every member of a workspace
cargo-reorder --all

# Reorder only specific packages
cargo-reorder -p foo -p bar

# CI mode: exit 1 if anything would change
cargo-reorder --check --all

# Run `cargo fmt` first, then reorder
cargo-reorder --fmt --all
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
