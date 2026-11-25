//! The `sims-content` integration tests, one module per subject, built as one
//! test binary. `common` builds synthetic installs and archives.
//!
//! Each module was its own test binary before; a test's full name is now
//! `<module>::<test>` (`cargo test -p sims-content --test integration <module>::`).

mod animations;
mod buy_catalog;
mod catalog;
mod common;
mod family_import;
mod install;
mod neighborhood;
mod surfaces;
mod ui_graphics;
mod wardrobe;
