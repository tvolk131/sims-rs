//! The VM against real game data, when `SIMS_INSTALL` names an install,
//! and against synthetic lots and stores otherwise.
//!
//! One test binary holds every integration suite, one module per former
//! `tests/*.rs` file, so the workspace links one executable rather than
//! fourteen. `tests/interpreter` stays its own binary: it needs no game data.
//! The modules share [`common`], which builds what several suites read
//! but none changes (the install's index, catalog and linked program) once
//! per process.
//!
//! Each module's temporary files are named by module, process and a
//! counter, so the suites run side by side in one process.

mod common;

mod boot;
mod build_terrain_pool_roof;
mod caf;
mod cheats;
mod coop;
mod destination_lots;
mod fuzz;
mod generic_routines;
mod interaction_sweep;
mod neighborhood_lifecycle;
mod phone_go_downtown;
mod program;
mod resize;
mod save;
mod travel;
