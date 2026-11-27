//! `sims-game`'s integration tests, built as one binary.
//!
//! Each module was a `tests/<name>.rs` binary of its own. The screenshot
//! ratchet (`tests/ratchet.rs`, `harness = false`) and the UI oracle tour
//! (`tests/ui_oracle_tour.rs`) stay binaries of their own.

mod cheats_cache_and_open_pack;
mod common;
mod dialog_timer;
mod downtown_arrival;
mod goth;
mod magic_town_arrival;
mod native_hotkeys;
mod neighborhood_shell;
mod options_and_dialogs;
mod perf_part4;
mod phone_go_downtown;
mod playtest_regressions;
mod polish_tryon_books_nb_and_fast_saves;
mod save;
mod sound;
mod special_panels_camera_animations;
mod trip_requests_and_neighborhood_iff;
mod tuning;
mod ui_polish;
mod ui_wave2_snapshots;
mod wave1_integration;
mod wave2_integration;
mod wave3_coop_integration;
