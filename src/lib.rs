//! Reorder top-level items in Rust source files following Rust community style.
//!
//! The default order applied by this crate is (each numbered slot is
//! a separate sort bucket; see README for the per-flag rationale):
//!
//! 1. `extern crate` declarations
//! 2. `mod` declarations (non-test) — pass `--no-mod-before-use` to
//!    flip slots 2 and 3 (i.e. put `use` first)
//! 3. `use` imports (grouped: std/core/alloc → external → crate/self/super)
//! 4. `pub use` re-exports (same grouping rules as imports)
//! 5. `const` items
//! 6. `static` items
//! 7. type aliases (`type X = ...`)
//! 8. `trait` / `trait alias` definitions — pass `--no-trait-before-struct`
//!    to push these after #9 / #10 instead
//! 9. `enum` definitions
//! 10. `struct` / `union` definitions
//! 11. `impl` blocks (kept after the type they implement when reasonable)
//! 12. `extern { ... }` foreign blocks
//! 13. free `fn` (synchronous)
//! 14. free `async fn`
//! 15. macro definitions (`macro_rules!` / `macro`)
//! 16. `#[cfg(test)] mod tests` (always last)
//!
//! Comments and blank-line separators are preserved as leading trivia on each
//! item, so reordering is safe to apply automatically.

pub mod diagnostic;
pub mod discover;

mod emit;
mod fields;
mod frontmatter;
mod imports;
mod macros;
mod reorder;
mod text;

pub use reorder::{
    Config, ReorderError, reorder_source, reorder_source_with, reorder_source_with_path,
};
