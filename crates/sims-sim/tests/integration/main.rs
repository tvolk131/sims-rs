//! The `sims-sim` integration tests, one module per subject, built as one
//! test binary.
//!
//! Each module was its own test binary before; a test's full name is now
//! `<module>::<test>` (`cargo test -p sims-sim --test integration <module>::`).

mod album_store;
mod animator;
mod autosave;
mod load;
mod motives;
mod neighborhood_save;
mod neighborhood_store;
mod neighbors;
mod patch_import;
mod pose;
mod rooms;
mod state_fuzzer;
mod world;
