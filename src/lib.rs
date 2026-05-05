//! Reorder top-level items in Rust source files following Rust community style.
//!
//! The recommended item order applied by this crate is:
//!
//! 1. `extern crate` declarations
//! 2. `use` imports (grouped: std/core/alloc -> external -> crate/self/super)
//! 3. `pub use` re-exports (same grouping rules as imports)
//! 4. `mod` declarations (non-test)
//! 5. `const` items
//! 6. `static` items
//! 7. type aliases (`type X = ...`)
//! 8. `enum` definitions
//! 9. `struct` / `union` definitions
//! 10. `trait` / `trait alias` definitions
//! 11. `impl` blocks (kept after the type they implement when reasonable)
//! 12. free `fn` (synchronous)
//! 13. free `async fn`
//! 14. macro definitions (`macro_rules!` / `macro`)
//! 15. `#[cfg(test)] mod tests` (always last)
//!
//! Comments and blank-line separators are preserved as leading trivia on each
//! item, so reordering is safe to apply automatically.

pub use reorder::{
    Config, ReorderError, reorder_source, reorder_source_with, reorder_source_with_path,
};

pub mod diagnostic;
pub mod discover;
pub mod imports;
pub mod reorder;

mod emit;
mod frontmatter;
mod macros;
mod text;
