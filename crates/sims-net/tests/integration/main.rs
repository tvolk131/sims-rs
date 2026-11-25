//! The `sims-net` integration tests, one module per subject, built as one
//! test binary.
//!
//! Each module was its own test binary before; a test's full name is now
//! `<module>::<test>` (`cargo test -p sims-net --test integration <module>::`).

mod iroh_loopback;
mod lockstep_memory;
