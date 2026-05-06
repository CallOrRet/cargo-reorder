# cargo-reorder

> [English](./README.md) · 中文

按照常见的 Rust 社区写法，重新排列 Rust 源文件里顶层 item 的顺序。

`rustfmt` 和 Rust Style Guide 都**没有规定**顶层 item 之间的相对顺序，本工具实现的是社区里常见的一种约定，并通过命令行参数让你切换到自己项目偏好的变体。

## 默认排序

1. `extern crate` 声明
2. `use` import —— 分三组，组间空一行：
   - **std**：`std` / `core` / `alloc`
   - **external**：第三方 crate
   - **crate-local**：`crate` → `super` → `self` → local-mod
     （local-mod 指 `use foo::...` 的首段名字匹配本文件里的 `mod foo;`；这一组内部不再插空行）
3. `pub use` re-export（同样的三组细分）
4. `mod` 声明 *（可用 `--mod-before-use` 切换到「mod 在 use 之前」的少数派写法，见下文）*
5. `const`
6. `static`
7. 类型别名 `type X = ...`
8. `trait` —— 紧跟那些目标类型不在本文件里的 `impl Trait for X`（`X` 不本地，所以 impl 锚定到 trait 而不是 type）。**（用 `--struct-before-trait` 与 #9 / #10 互换位置。）**
9. `enum` —— 紧跟它的 `impl` 块
10. `struct` / `union` —— 紧跟它的 `impl` 块
11. 无锚点 `impl` —— target 类型和 trait 都不在本文件 name_index 里（比如 `submod.rs` 实现的 impl，目标 type 定义在 `lib.rs`；不是 Rust 「孤儿」语义，是 parser 一次只看一个文件造成的）
12. `extern { ... }` foreign block
13. `fn`
14. `async fn`
15. 宏定义（`macro_rules!`）
16. `#[cfg(test)] mod tests` —— 永远放最后

每个类型自己的 `impl` 组内部按 **inherent → std trait → crate-local trait → 第三方 trait** 排序。trait 分类有三条线索：

1. **路径首段**：`std::*` / `core::*` / `alloc::*` → std；`crate::*` / `self::*` / `super::*` → crate-local。
2. **文件里的 `use` 导入（含 `as` 别名）**：`use std::fmt::Display;` 或 `use std::fmt::Display as D;` 让 `impl Display for Foo` / `impl D for Foo` 被认作 std-trait；`use crate::MyTrait as M;` 同理把 `impl M for Foo` 算作 crate-trait。
3. **本文件 trait 声明**：`trait Foo {}` 直接写在本文件顶层时，`impl Foo for X` 不需要 `use` 也算 crate-trait。
4. **Rust prelude**：编译器自动 import 的 34 个 trait（取自 [`library/std/src/prelude/mod.rs`](https://github.com/rust-lang/rust/blob/master/library/std/src/prelude/mod.rs) 的 v1 + rust_2021 + rust_2024 并集）—— marker（`Send`/`Sync`/`Sized`/`Unpin`/`Copy`）、`Fn`/`AsyncFn` 全家、`Drop`、`Clone`/`Default`/`ToOwned`、cmp 四件套、`From`/`Into`/`TryFrom`/`TryInto`/`AsRef`/`AsMut`、iter 全家、`ToString`、还有 2024 加进来的 `Future`/`IntoFuture`。这些不需要显式 `use` 也分类成 std。不在 prelude 里的 std trait（`Display` / `Debug` / `Read` / `Write` / `Add` 等）还是要 import 才识别（这些 trait 不 import 本来 Rust 也编不过）。

既没前缀、又没 import、又不在本文件 trait 声明里、又不在 prelude 的单段名落到 external（保守，符合 Rust 实际作用域）。

其他类别内部保持原始相对顺序（stable sort）。

import 块的边界（不同 `(category, visual_group)` 之间，以及最后一个 import 块和第一个非 import item 之间）会自动插入一行空白。非 import 之间的空行布局不动，保留用户原有的格式。

## 默认排序 —— 完整示例

输入（顺序很乱的源码）：

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

跑 `cargo-reorder`（默认配置）后（输出**完全是工具实跑结果**，不是手工排版）：

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

要点：

* `//! crate docs` 和 `#![allow(dead_code)]` 留在文件最顶部 —— 文件级的内容永远不会被打乱。
* `extern crate alloc;` 排到最前面，和 `use` 块之间空一行。
* `use` 拆成 std / external / crate-local 三个视觉组，组间空行。第三组内部按 `crate` → `super` → `self` → local-mod 的顺序；这里 `helpers` 因为有同文件里的 `mod helpers;` 声明，被识别为 local-mod。
* `pub use` 同样按这套子分组，但单独成一块，紧跟在 `use` 之后。
* 这个示例里没有 `macro_rules!`。如果有,每个宏 item 都会作为 barrier(见下文「宏 item 是硬 barrier」),会把周围的排序切成独立的"barrier 之上 / barrier 之下"两段。
* `struct Foo` 后面贴着它的三个 impl，顺序是 inherent (`impl Foo`) → std trait (`impl Display for Foo`) → 第三方 trait (`impl other_crate::Trait for Foo`)。
* `trait Greet` 也带走了 `impl Greet for u32` —— 因为目标类型 `u32` 不在本文件，impl 锚定到 trait 而不是类型。
* `#[cfg(test)] mod tests` 不论原文里在哪，最终都在文件末尾。

## 命令行参数

按功能分组。

### 文件发现范围

| 参数 | 作用 |
| --- | --- |
| `--all` | 处理 workspace 里所有成员（对齐 `cargo fmt --all`） |
| `-p`, `--package <NAME>` | 只处理指定的 package（对齐 `cargo fmt -p NAME`），可重复 |
| `--manifest-path <PATH>` | 指定 `Cargo.toml` 的路径用于发现文件 |
| `--exclude <substr>` | 路径包含该子串就跳过 |

### Item 排序策略

| 参数 | 作用 |
| --- | --- |
| `--pub-mod-first` | `pub mod` 排在 `mod` 之前并加空行（见「关于 `--pub-mod-first`」） |
| `--mod-before-use` | `mod` 排在 `use` 之前（少数派写法，见「关于 `--mod-before-use`」） |
| `--struct-before-trait` | `enum` / `struct` 排在 `trait` 之前（默认 trait-first；见「关于 `--struct-before-trait`」） |
| `--no-import-groups` | 不分 std / external / crate 三组 |
| `--no-impl-grouping` | 不让 impl 跟随它的 type，所有 impl 一桶 |
| `--no-tests-last` | 不强制 `#[cfg(test)] mod tests` 放最后 |
| `--no-reorder-inline-mods` | 不重排 inline `mod foo { ... }` 的 body（见「关于 `--no-reorder-inline-mods`」） |

### 输出模式

| 参数 | 作用 |
| --- | --- |
| `--check` | CI 模式：有变化就退出码 1 |
| `--stdout` | 写到 stdout 不修改文件 |
| `-v`, `--verbose` | 打印每个被改写的文件名（默认是静默） |
| `--color <auto\|always\|never>` | 给 `--check` 的 diff 上色（`-` 红、`+` 绿、头部青）和 parse error 上色。默认 `auto`：stderr 是 tty 且没有 `NO_COLOR` 环境变量时才上色。 |

> 下面的数据是用 `sample-stats`（本仓库 `src/bin/sample-stats.rs` 里的 `syn` 解析二进制）跑 fresh clone 的项目得到。换 `syn` 替代 `grep` 之后，所有可见性变体（`pub` / `pub(crate)` / `pub(super)` / `pub(in path)`）和 inline `mod foo { ... }` 嵌套都被正确识别。每个 scope（文件主体 OR 一个非 test 的 inline `mod` 主体）算一次观察；**test mod**（名为 `tests` / `test`，或带 `#[cfg(test)]` 的）整体跳过 —— 不算自己的观察，也不递归进去。

### 复现这些数据

`sample-stats` 是本 crate 的第二个 binary。编译一次，传任意多个项目根目录即可：

```shell
cargo build --release --bin sample-stats

# 单个项目：
./target/release/sample-stats ~/src/serde

# 多个一起跑 —— 合计数据跨所有项目聚合：
./target/release/sample-stats \
    ~/src/anyhow ~/src/serde ~/src/clap ~/src/regex \
    ~/src/syn ~/src/ripgrep ~/src/tracing ~/src/tokio ~/src/cargo
```

每个项目的进度信息打到 **stderr**；三张 Markdown 表（mod-vs-use、pub/priv mod 分布、trait-vs-struct）打到 **stdout**，可以直接管道写进文档：

```shell
./target/release/sample-stats ~/src/* > stats.md
```

每个目录下所有 `.rs` 文件都用 `syn::parse_file` 解析（自动跳过 `target/` 和 `.git/`）。parse 失败的文件在 stderr 那行的 `parse-skipped` 里计数，不参与观察。**没有任何 flag** —— 所有观察规则（跳过 test mod、递归 inline mod、可见性识别）都写死在源码里。

## 关于 `--pub-mod-first`

统计同时含 `pub mod foo;` 和私有 `mod foo;` 声明的 scope（只看外部声明，不算 inline `mod foo { ... }` 块）—— 9 个项目共 64 个 scope：

| 项目 | pub-first | priv-first | interleaved |
| --- | --: | --: | --: |
| ripgrep | 0 | 0 | 1 |
| serde | 4 | 2 | 0 |
| syn | 1 | 0 | 1 |
| clap | 1 | 4 | 2 |
| regex | 2 | 1 | 5 |
| tracing | 1 | 3 | 3 |
| cargo | 2 | 2 | 10 |
| tokio | 3 | 2 | 14 |

合计：**22% pub-first，22% priv-first，56% interleaved**。默认保留源序 —— `pub mod` 和 `mod` 不按可见性重新洗牌。

## 关于 `--mod-before-use`

**没有官方规则规定 `mod` 一定要在 `use` 前面或后面**。统计同时含外部 `mod foo;` 声明和 `use ...;` 的 scope（inline `mod foo { ... }` 块不计入"mod 声明"，它的 body 作为独立 scope 单独采样）—— 9 个项目共 254 个：

| 项目 | use-first | mod-first |
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

合计：**76% use-first，24% mod-first**。默认 use-first；项目偏 mod-first 时（典型如 `syn`，大量 inline 子模块内部全是 mod-first；以及 `tokio` 在工作区层面整体偏 mod-first）加 `--mod-before-use`。

## 关于 `--struct-before-trait`

统计同时含 `trait` 和 `struct` / `enum` / `union` 声明的 scope —— 9 个项目共 105 个：

| 项目 | trait-first | struct-first |
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

合计：**87% trait-first，13% struct-first** —— 强烈 trait-first。大部分 crate 内部抽象用 `pub(crate) trait` 写在实现它的 struct **之前**。默认 trait-first；项目逆这个潮流时再加 `--struct-before-trait`。

## 关于 `--no-reorder-inline-mods`

默认 cargo-reorder 会递归进入 inline `mod foo { ... }`,用同一套规则处理它的 body(再深一层的 inline mod 也会递归)。加 `--no-reorder-inline-mods` 之后只排**文件顶层**的 item,所有 inline mod 的 body 保持字节不动。

**有意跳过**的三种 mod —— 它们的 item 顺序属于公共契约或影响编译语义：

| 模式 | 跳过原因 |
| --- | --- |
| `#[cfg(test)] mod ...` / `mod tests { ... }` | 已被 `--tests-last` 拉到文件末尾；测试夹具的顺序往往是叙事性的，重排会掩盖意图 |
| `#[macro_use] mod ...` | 里面定义的 `macro_rules!` 会泄露到父作用域，body 内重排会改变可见性顺序 |
| 纯 `use` mod（所有 item 都是 `use ...`） | 覆盖 `prelude`、`__private`、sealed-trait re-export 等场景 —— 这种顺序就是公共 API 的一部分 |

只要 body 里**有一个非-`use` 的 item**,这个 mod 就是合格目标。inline mod body 内的 `macro_rules!` 同样作为 barrier 处理 —— 在 body 内部钉位,body 内任何其他 item 都不能跨过它。

默认关，因为 `prelude` 风格 mod 和 codegen 脚手架在生态里很常见，意外重排它们的代价比"常规 inline mod 重排"的收益大。建议先在代表性文件上看完 diff 再全项目开。

## 注释和属性

注释和属性都被保留：

* doc 注释（`///`、`//!`）和 `#[...]` 属性始终跟随对应的 item
* item 正上方紧贴的 `//` 或 `/* */` 注释跟随该 item 移动
* 如果某个注释块和下面的 item 之间隔了空行，则视为上一个 item 的尾部内容；如果它在第一个 item 之前，则视为文件级的头注释,留在文件顶部
* **前后都被空行包围的 `//` 注释块视为「浮动栅栏」**:它原地不动,且栅栏上方的 item 不允许被排到栅栏下方(反之亦然)。适合保留手写的章节分隔,例如:
  ```rust
  // === public API ===

  pub fn ...

  // === helpers ===

  fn ...
  ```
  每个段落独立排序,分隔线本身不挪。doc 注释(`///`、`//!`)不属于栅栏 —— syn 会把它们当成下一个 item 的 attribute。
* shebang（`#!/usr/bin/env ...`）和 crate 级内部属性（`#![...]`）始终留在文件最顶部

## 用法

```shell
# 默认：跑 `cargo metadata` 发现当前 package 的所有 target，沿 mod 树重排
cargo-reorder

# 整个 workspace
cargo-reorder --all

# 指定 package
cargo-reorder -p foo -p bar

# 单个文件 / 目录（绕过 cargo metadata）
cargo-reorder src/lib.rs
cargo-reorder src/

# stdin → stdout 过滤模式
cargo-reorder < input.rs > output.rs

# CI 检查模式
cargo-reorder --check
```

也可以作为 cargo 子命令调用：

```shell
cargo reorder
cargo reorder --all
```

## 已验证

在多个流行的真实 crate 上做了端到端验证。每个项目流程：clone 新的源码 → 跑 `cargo check` 和 `cargo test` 记录 baseline → 用本工具重排所有文件 → 再跑一次 `cargo check` 确认编译通过 → 再跑测试对比。

| 项目 | 总 .rs | LOC | 重排数 | 用时 | 编译 | 测试 |
| --- | --: | --: | --: | --: | :-: | :-: |
| [anyhow](https://github.com/dtolnay/anyhow) | 37 | 5,833 | 21 | 0.11 s | ✅ | ✅（3）|
| [serde](https://github.com/serde-rs/serde) | 208 | 42,630 | 42 | 0.23 s | ✅ | ✅（5）|
| [ripgrep](https://github.com/BurntSushi/ripgrep) | 100 | 52,266 | 42 | 0.46 s | ✅ | *（无 `[lib]` target）|
| [syn](https://github.com/dtolnay/syn) | 133 | 68,988 | 71 | 0.59 s | ✅ ** | —（测试 suite 依赖 `rustc-dev`，仅 nightly）|
| [tracing](https://github.com/tokio-rs/tracing) | 260 | 71,547 | 123 | 0.51 s | ✅ | ✅（188）|
| [clap](https://github.com/clap-rs/clap) | 329 | 83,179 | 80 | 0.47 s | ✅ | ✅ |
| [regex](https://github.com/rust-lang/regex) | 225 | 159,330 | 83 | 1.38 s | ✅ | ✅（7）|
| [tokio](https://github.com/tokio-rs/tokio) | 777 | 173,843 | 254 | 0.65 s | ✅ | ✅（lib，146）|
| [cargo](https://github.com/rust-lang/cargo) | 1,352 | 333,542 | 782 | 2.42 s | ✅ | ✅（lib，160）|

`*` ripgrep 是 binary crate（没有 `[lib]` target），所以 `cargo test --lib` 不适用，但 `cargo check --all-targets` 重排后通过。
`**` syn 编译失败和 baseline 完全一致 —— 它的测试 suite 强制要 `--all-features`，引入了只在 nightly `rustc-dev` 组件里的 internal crate。不是我们工具搞坏的。

数据：release 二进制单线程跑在 fresh clone 的项目上。"重排数" 是真有改动的文件数，其他文件本来就在规范顺序。

工具在每个项目上也是**幂等**的：第一次重排后再跑一次 `--check`，没有任何进一步的 diff 报出来。

### 测试集结构

按主题一个文件一种关注点：

| 文件 | 覆盖内容 |
| --- | --- |
| `tests/items.rs` | 16 种顶层 item 各自归到正确分类 |
| `tests/imports.rs` | `extern crate` / `use` / `pub use` 分组和别名 |
| `tests/impls.rs` | impl 锚定 + inherent → std → crate → external 四档排序 |
| `tests/macros.rs` | 宏 item 作为 barrier 的语义:钉位、段隔离、idempotent |
| `tests/cross_file.rs` | `mod foo;` 文件查找、`#[path]` 重定向、子文件缺失 fallback |
| `tests/comments.rs` | leading 注释、内部 doc、文件头注释块 |
| `tests/attributes.rs` | `#[derive]` / `#[cfg]` / `#[cfg_attr]` / 多行属性 |
| `tests/generics.rs` | 生命周期、where 子句、const 泛型、GAT、HRTB、async trait |
| `tests/visibility.rs` | `pub` / `pub(crate)` / `pub(super)` / `pub(in path)` 往返 |
| `tests/flags.rs` | 所有 `Config` flag 端到端 |
| `tests/frontmatter.rs` | RFC 3502 cargo-script frontmatter |
| `tests/idempotence.rs` | 复杂代表性文件 round-trip |
| `tests/edge_cases.rs` | unicode / 原始字符串 / 多空行 / inline mod / EOF 无换行 |
| `tests/discover.rs` | `cargo metadata` 文件发现、`--all` / `-p` / `--manifest-path` |

### 验证过程中发现的注意点

* **宏 item 是硬 barrier。** `macro_rules!` 是文本作用域,且调用点形态多(struct 字段初始化、类型注解、深埋在 sibling 文件里通过 `#[macro_use] mod` 漏出来 …)。与其每种形态都试图正确识别 caller,工具直接把所有宏相关的顶层 item 钉在源码位置,**禁止任何其他 item 跨越**。barrier 集合:
  - `macro_rules! foo`(无论有没有 ident)—— 钉自己
  - `lazy_static! { ... }` / `to_hash_map!(...)` 这类裸顶层宏调用 —— 钉自己
  - `#[macro_use] mod foo;` —— 钉自己,因为它从这一行开始把 child 的 `macro_rules!` 漏到父 scope

  代价:含多个宏的文件会被切成多个独立排序段,有时排得没那么彻底。换来的是规则极其简单、永远不会产生编译不过的输出。
* **支持 RFC 3502 cargo-script 文件。** 开头带 `---` ... `---` TOML frontmatter 的单文件脚本（前面可以有 shebang）会被识别：frontmatter 原样保留，只重排 Rust 主体部分。`---` 裸开头和 `---cargo` 带 info string 两种形式都支持。
* **构建文件 parse 错误严格、非构建文件静默跳过。** 对齐 `cargo fmt`：从 cargo target 的 `src_path` 沿 mod 树能走到的文件（也就是项目真的会编译的文件）按严格处理 —— parse 错误会打印 rustc 风格诊断并以非零状态码退出。**不在**构建树里的文件（典型如 cargo 自己的 `tests/testsuite/script/rustc_fixtures/`、rustfix 的 `tests/everything/`，以及其他扩展名是 `.rs` 但不参与编译的测试夹具）parse 失败时静默跳过。这个判断用的是和默认发现模式同一份 `cargo metadata` 输出，所以无论是 `cargo-reorder` 不带参数、加 `--all`、还是给显式路径，都能正确分类。

## License

Apache-2.0
