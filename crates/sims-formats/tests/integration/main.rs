//! The `sims-formats` integration tests, one module per subject, built as one
//! test binary. `avatar_codecs` keeps its own submodules.
//!
//! Each module was its own test binary before; a test's full name is now
//! `<module>::<test>` (`cargo test -p sims-formats --test integration <module>::`).

mod avatar_codecs;
mod boundaries;
mod family_file;
mod iff_write;
mod neighborhood_file;
mod object_functions;
