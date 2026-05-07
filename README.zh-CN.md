# cargo-reorder

> [English](./README.md) · 中文

按照常见的 Rust 社区写法，重新排列 Rust 源文件里顶层 item 的顺序。

`rustfmt` 和 Rust Style Guide 都**没有规定**顶层 item 之间的相对顺序，本工具实现的是社区里常见的一种约定，并通过命令行参数让你切换到自己项目偏好的变体。

## 默认排序

1. `extern crate` 声明
2. `mod` 声明 *（用 `--no-mod-before-use` 把 `use` 提到 `mod` 前面，见下文）*
3. `use` import —— 分三组，组间空一行：
   - **std**：`std` / `core` / `alloc`
   - **external**：第三方 crate
   - **crate-local**：`crate` → `super` → `self` → local-mod
     （local-mod 指 `use foo::...` 的首段名字匹配本文件里的 `mod foo;`；这一组内部不再插空行）
4. `pub use` re-export（同样的三组细分）
5. `const`
6. `static`
7. 类型别名 `type X = ...`
8. `trait` —— 紧跟那些目标类型不在本文件里的 `impl Trait for X`（`X` 不本地，所以 impl 锚定到 trait 而不是 type）。**（用 `--no-trait-before-struct` 与 #9 / #10 互换位置。）**
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

要点：

* `//! crate docs` 和 `#![allow(dead_code)]` 留在文件最顶部 —— 文件级的内容永远不会被打乱。
* `extern crate alloc;` 排到最前面，和后续之间空一行。
* `mod` 声明紧跟其后（默认 mod 在前；加 `--no-mod-before-use` 翻转）。
* `use` 拆成 std / external / crate-local 三个视觉组，组间空行。第三组内部按 `crate` → `super` → `self` → local-mod 的顺序；这里 `helpers` 因为有同文件里的 `mod helpers;` 声明，被识别为 local-mod。
* `pub use` 同样按这套子分组，但单独成一块，紧跟在 `use` 之后。
* 这个示例里没有 `macro_rules!`。如果有,每个宏 item 都会作为 barrier(见下文「宏 item 是硬 barrier」),会把周围的排序切成独立的"barrier 之上 / barrier 之下"两段。
* `struct Foo` 后面贴着它的三个 impl，顺序是 inherent (`impl Foo`) → std trait (`impl Display for Foo`) → 第三方 trait (`impl other_crate::Trait for Foo`)。
* `trait Greet` 也带走了 `impl Greet for u32` —— 因为目标类型 `u32` 不在本文件，impl 锚定到 trait 而不是类型。
* `#[cfg(test)] mod tests` 不论原文里在哪，最终都在文件末尾。

## 命令行参数

按功能分组。

### 文件发现范围

文件选择参数刻意收缩到 `cargo fmt` 的子集——同样的参数组合产出**同一份**文件清单。**不支持直接传文件或目录**（cargo fmt 也不支持），按 package 维度操作即可。

| 参数 | 作用 |
| --- | --- |
| `-p`, `--package <NAME>` | 只处理指定的 package（对齐 `cargo fmt -p NAME`），可重复 |
| `--all` | 处理 workspace 里所有成员（对齐 `cargo fmt --all`） |
| `--manifest-path <PATH>` | 指定 `Cargo.toml` 的路径用于发现文件（对齐 `cargo fmt --manifest-path`） |

### Item 排序策略

| 参数 | 作用 |
| --- | --- |
| `--fn-args` | 重排函数参数；默认关闭，因为参数顺序属于调用契约 |
| `--no-fields` | 不对 `struct` / `union` / `enum` 内部的具名字段做"前缀分组+长度排序"（默认开启，见「关于 `--no-fields`」） |
| `--no-impl-fns` | 保留 `impl` / `trait` 块内成员（`const` / `type` / `fn` / `async fn`）的源序，字段级和顶层级排序仍然生效 |
| `--no-tests-last` | 不强制 `#[cfg(test)] mod tests` 放最后 |
| `--no-inline-mods` | 不重排 inline `mod foo { ... }` 的 body（见「关于 `--no-inline-mods`」） |
| `--no-impl-grouping` | 不让 impl 跟随它的 type，所有 impl 一桶 |
| `--no-import-groups` | 不分 std / external / crate 三组 |
| `--no-mod-before-use` | `use` 排在 `mod` 之前（默认 mod 在前，见「关于 `--no-mod-before-use`」） |
| `--no-short-trait-first` | 不让短 trait 路径优先排序（默认开启，例如 `impl Default for Foo` 排在 `impl std::default::Default for Foo` 之前） |
| `--no-trim-field-blanks` | 保留多行字段重排时原本存在的字段间空行（默认删除；排到第一位的字段前置空行始终删除） |
| `--no-preserve-mod-order` | `pub mod` 排在 `mod` 之前并加空行（见「关于 `--no-preserve-mod-order`」） |
| `--no-single-line-fields` | 保留单行 `struct` / `union` / `enum` 字段、struct 初始化字段的源序（默认会原地重排） |
| `--no-trait-before-struct` | `enum` / `struct` 排在 `trait` 之前（默认 trait-first；见「关于 `--no-trait-before-struct`」） |

### 输出模式

| 参数 | 作用 |
| --- | --- |
| `-v`, `--verbose` | 打印每个被改写的文件名（默认是静默） |
| `--fmt` | reorder 之前先跑一遍 `cargo fmt`（沿用同样的 `-p` / `--all` / `--manifest-path`）。`--check` 模式下自动跳过，避免 CI gate 写盘。 |
| `--check` | CI 模式：有变化就退出码 1 |
| `--color <auto\|always\|never>` | 给 `--check` 的 diff 上色（`-` 红、`+` 绿、头部青）和 parse error 上色。默认 `auto`：stderr 是 tty 且没有 `NO_COLOR` 环境变量时才上色。 |

> 下面的数据是用 `sample-stats`（本仓库 `src/bin/sample-stats.rs` 里的 `syn` 解析二进制）跑 fresh clone 的项目得到。换 `syn` 替代 `grep` 之后，所有可见性变体（`pub` / `pub(crate)` / `pub(super)` / `pub(in path)`）和 inline `mod foo { ... }` 嵌套都被正确识别。每个 scope（文件主体 OR 一个非 test 的 inline `mod` 主体）算一次观察；**test mod**（名为 `tests` / `test`，或带 `#[cfg(test)]` 的）整体跳过 —— 不算自己的观察，也不递归进去。

### 复现这些数据

`sample-stats` 是本 crate 的第二个 binary。编译一次，传任意多个项目根目录即可：

```shell
cargo build --release --bin sample-stats

# 单个项目：
./target/release/sample-stats ~/src/serde

# 多个一起跑 —— 每个项目在合计里投一票：
./target/release/sample-stats \
    ~/src/anyhow ~/src/axum ~/src/bat ~/src/bevy ~/src/cargo \
    ~/src/chrono ~/src/clap ~/src/diesel ~/src/hyper ~/src/itertools \
    ~/src/rayon ~/src/regex ~/src/reqwest ~/src/ripgrep ~/src/rust-analyzer \
    ~/src/serde ~/src/syn ~/src/thiserror ~/src/tokio ~/src/tower ~/src/tracing
```

每个项目的进度信息打到 **stderr**；三张 Markdown 表（mod-vs-use、pub/priv mod 分布、trait-vs-struct）打到 **stdout**，可以直接管道写进文档：

```shell
./target/release/sample-stats ~/src/* > stats.md
```

每个目录下所有 `.rs` 文件都用 `syn::parse_file` 解析。目录层面跳过：`target/`、`.git/`，以及惯例上的非库目录 `tests/`、`examples/`、`benches/` —— 它们走集成测试那套约定（`mod common;` 共享 helper、derive-UI 用的临时 struct、bench 一次性代码），并不反映项目库源码的真实组织风格。parse 失败的文件在 stderr 那行的 `parse-skipped` 里计数，不参与观察。**没有任何 flag** —— 所有观察规则（跳过 test mod、递归 inline mod、可见性识别）都写死在源码里。

## 关于 `--no-preserve-mod-order`

每个 scope 用 pair-majority 投票：把 (pub mod, priv mod) 所有配对枚举一遍，看是 pub 在前还是 priv 在前更多——21 个项目里有 19 个有合资格的 scope：

| 项目 | pub-first | priv-first | tied |
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

按项目聚合（每个项目按其内部多数投一票，平局单独计）：**10/19 pub-first (53%)，5/19 priv-first (26%)，4/19 tied (21%)**。略偏 pub-first。默认仍保留源序 —— 默认开启会让那些偏 priv-first 的项目（约 26%）和持平项目（约 21%）的输出无声变化。项目内部偏 pub-first 时再加 `--no-preserve-mod-order`。

## 关于 `--no-mod-before-use`

**没有官方规则规定 `mod` 一定要在 `use` 前面或后面**——rustfmt 也没立场。统计同时含外部 `mod foo;` 声明和 `use ...;` 的 scope（inline `mod foo { ... }` 块不计入"mod 声明"，它的 body 作为独立 scope 单独采样）—— 21 个项目共 618 个 scope。每个 scope 用 pair-majority 投票（数对 (mod, use) 中谁在前的多数胜出，平局算 tied）：

| 项目 | use-first | mod-first | tied |
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

按项目聚合（每个项目按内部多数投一票，平局单独计）：**7/21 use-first (33%)，12/21 mod-first (57%)，2/21 tied (10%)**。**默认按多数走，mod 在前**。项目内部偏 use-first 时（典型如 `regex`、`ripgrep`、`cargo`、`chrono`、`tracing`、`rayon`、`reqwest`）加 `--no-mod-before-use`。

## 关于 `--no-trait-before-struct`

统计同时含 `trait` 和 `struct` / `enum` / `union` 声明的 scope —— 21 个项目里有 20 个有合资格的 scope。每个 scope 用 pair-majority 投票：

| 项目 | trait-first | struct-first | tied |
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

按项目聚合（每个项目按内部多数投一票，平局单独计）：**14/20 trait-first (70%)，3/20 struct-first (15%)，3/20 tied (15%)**。强烈 trait-first。默认 trait-first；项目逆这个潮流时再加 `--no-trait-before-struct`。

## 关于 `--no-fields`

默认情况下 cargo-reorder 在**四个层面**用同一套分组规则：

- **字段级**：每个 `struct` / `union` 的具名字段、每个 `enum` 的变体、以及 struct-like 变体内部的具名字段
- **顶层（同类内部）**：连续的顶层 `struct` / `union` / `enum` / `trait` / `fn` / `async fn` 之间，同类 item 按同样的"前缀分组 + 长度"规则重排。`impl` 块跟随它锚定的类型一起移动，所以 `struct Foo` + `impl Foo` 始终连在一起
- **`impl` / `trait` 块内部**：成员按与顶层一致的分类顺序排——`const` → `type` → `fn` → `async fn`，每一类内部再用前缀分组 + 长度规则。宏 / verbatim 成员是硬屏障，整个 body 保持源序
- **struct 初始化表达式**：`S { ... }`、`U { ... }`、`E::V { ... }` 内的具名字段也按同一规则重排。functional update 的 `..base` 保持在末尾

四个层面共用同一套分组顺序：

1. **按"首词"分组**：
   - snake_case：取第一个 `_` 之前的串（`foo_bar` → `foo`）
   - PascalCase / camelCase：取第一个小写→大写的转折之前（`FooBar` → `Foo`，`fooBar` → `foo`）
   - 没有 `_` 也没有大小写转折的（`Foo`、`BAR`、`XMLParser`）自成一组
2. **组内**按名字长度升序，短的在前（`bar_x` 在 `bar_long_name` 之前），同长度按原顺序
3. **组间**按**该组的平均名字长度**升序（所有成员名字长度的算数平均），同长度按原顺序

示例（左为输入，右为输出）：

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

加 `--no-fields` 同时关掉四层 —— 顶层 item 退回到"按 category，组内保留源序"，每个 `struct` / `union` / `enum` 的内部保持原样，每个 `impl` / `trait` body 和 struct 初始化表达式也保持原样。如果只想保留 `impl` / `trait` body 的源序、其他层照常排（比如方法是 builder 链、有特定调用顺序），用更精准的 `--no-impl-fns`。

单行字段列表默认也会用 byte/span 级 pass 处理，例如 `struct S { b: u8, a: u8 }`、`S { b: 1, a: 2 }`；输出仍保持单行，不插入空行分隔。加 `--no-single-line-fields` 可以只关闭这部分单行字段重排，同时保留多行字段重排。

多行字段类列表默认会删除字段之间已有的空行，并且不会新增组间空行。加 `--no-trim-field-blanks` 后，原有空行会跟随其后的字段一起移动。如果带有前置空行的字段被排到第一位，这些前置空行始终会被删除。

函数参数默认不重排。加 `--fn-args` 后，单行和多行参数列表都会使用同一套分组规则；第一个 receiver 参数（`self`、`mut self`、`&self`、`&mut self`）固定在最前面，其余普通 ident 参数会参与重排。多行签名会保留已有空行，但不会新增组间空行。

字段级 pass 会自动**跳过**那些重排会改 ABI / 内存布局 / 派生语义的形态：

| 模式 | 跳过原因 |
| --- | --- |
| 任何 `#[repr(...)]`（C、packed、transparent、align(N)、整型 repr） | 重排会改 ABI / 内存布局 |
| `#[derive(Ord)]` 或 `#[derive(PartialOrd)]` | 派生比较按字段/变体声明顺序读 |
| `enum` 任一变体带显式 discriminant（`A = 1`） | 重排会无声修改其它变体的隐式值 |
| 元组 struct / 元组变体 / 单元 struct / 单元变体 | 没有字段名可分组 |
| 字段数 < 2 | 没东西可排 |

这些跳过规则故意写得保守，目标是"默认开启字段重排始终是安全的"。如果你的代码里发现还有该跳过的场景，请提 issue。

## 关于 `--no-inline-mods`

默认 cargo-reorder 会递归进入 inline `mod foo { ... }`,用同一套规则处理它的 body(再深一层的 inline mod 也会递归)。加 `--no-inline-mods` 之后只排**文件顶层**的 item,所有 inline mod 的 body 保持字节不动。

**有意跳过**的三种 mod —— 它们的 item 顺序属于公共契约或影响编译语义：

| 模式 | 跳过原因 |
| --- | --- |
| `#[cfg(test)] mod ...` / `mod tests { ... }` | 默认就被 tests-last 规则拉到文件末尾(用 `--no-tests-last` 可关闭)；测试夹具的顺序往往是叙事性的，重排会掩盖意图 |
| `#[macro_use] mod ...` | 里面定义的 `macro_rules!` 会泄露到父作用域，body 内重排会改变可见性顺序 |
| 纯 `use` mod（所有 item 都是 `use ...`） | 覆盖 `prelude`、`__private`、sealed-trait re-export 等场景 —— 这种顺序就是公共 API 的一部分 |

只要 body 里**有一个非-`use` 的 item**,这个 mod 就是合格目标。inline mod body 内的 `macro_rules!` 同样作为 barrier 处理 —— 在 body 内部钉位,body 内任何其他 item 都不能跨过它。

默认开启 inline mod 递归。`prelude` 风格 mod 和 codegen 脚手架在生态里很常见,这些场景里的 body 顺序往往是 API 契约的一部分 —— 上面表格里的几类(`#[cfg(test)] mod`、`#[macro_use] mod`、纯 `use` mod)会被自动跳过 body 重排,对其他 inline mod 才递归。如果某个项目里还想完全关掉 inline mod 递归,加 `--no-inline-mods` 即可。

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
| `tests/impl_trait_body.rs` | `impl`/`trait` body 内成员重排（const → type → fn → async fn） |
| `tests/fields.rs` | struct/union 字段分组、enum 变体排序、前缀分组排序 |
| `tests/top_level_grouping.rs` | 顶层同类别 struct/enum/trait/fn 前缀分组 |
| `tests/macros.rs` | 宏 item 作为 barrier 的语义:钉位、段隔离、idempotent |
| `tests/cross_file.rs` | `mod foo;` 文件查找、`#[path]` 重定向、子文件缺失 fallback |
| `tests/inline_mods.rs` | inline `mod foo { ... }` 递归、skip-list、嵌套 inline mod |
| `tests/comments.rs` | leading 注释、内部 doc、文件头注释块 |
| `tests/floating_comments.rs` | 浮动注释围栏:检测、锚定、与排序的交互 |
| `tests/attributes.rs` | `#[derive]` / `#[cfg]` / `#[cfg_attr]` / 多行属性 |
| `tests/generics.rs` | 生命周期、where 子句、const 泛型、GAT、HRTB、async trait |
| `tests/visibility.rs` | `pub` / `pub(crate)` / `pub(super)` / `pub(in path)` 往返 |
| `tests/flags.rs` | 所有 `Config` flag 端到端 |
| `tests/filter_mode.rs` | 管道 stdin → stdout 模式、无文件发现路径 |
| `tests/fmt_flag.rs` | `--fmt` 委托 `cargo fmt` 并传递匹配的筛选参数 |
| `tests/frontmatter.rs` | RFC 3502 cargo-script frontmatter |
| `tests/idempotence.rs` | 复杂代表性文件 round-trip |
| `tests/edge_cases.rs` | unicode / 原始字符串 / 多空行 / inline mod / EOF 无换行 |
| `tests/discover.rs` | `cargo metadata` 文件发现、`-p` / `--all` / `--manifest-path` |

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
