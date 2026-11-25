//! The `sims-audio` integration tests, one module per subject, built as one
//! test binary.
//!
//! Each module was its own test binary before; a test's full name is now
//! `<module>::<test>` (`cargo test -p sims-audio --test integration <module>::`).

mod ambience;
mod common;
mod decoders;
mod install;
mod install_player;
mod install_tables;
mod levels;
mod music;
mod placement;
mod player;
mod radio;
mod tables;
mod voices;
