//! The `sims-antics` integration tests, built as one test binary with one module per
//! former test binary. A test's full name is now `<module>::<test>`
//! (`cargo test -p sims-antics --test integration <module>::`).
//!
//! Modules that need an optional feature are gated here rather than with `[[test]]
//! required-features`, so a `--no-default-features` build still runs the rest.

mod air_pad;
mod antics_test_cli;
#[cfg(feature = "vm-replay")]
mod antics_test_runner;
mod behavioral_equiv;
#[cfg(feature = "install-catalog")]
mod cli_lift;
mod common;
mod compiler_and_encoders;
mod dependency_weight;
mod equiv_pilot_upgrades;
mod equiv_round2;
mod equiv_round3;
mod install_roundtrip;
#[cfg(feature = "vm-replay")]
mod live_equiv;
#[cfg(feature = "vm-replay")]
mod live_equiv_corpus;
mod open_objects_vm;
mod open_pack_behavioral;
mod open_pack_household_sim;
#[cfg(feature = "install-catalog")]
mod structured_equiv;
mod v2_2_bitflags_and_return_error;
mod v2_2_lift_polish;
mod v2_2_lint_polish;
mod v2_2_step2_lint;
mod v2_2_temps_and_flags;
mod v2_3_if_let_and_fixes;
mod v2_3_lift_strict_leap;
mod v2_3_step2_node_order_and_tails;
mod v2_4_compiler_upgrades;
mod v2_4_dead_false;
mod v2_4_lift_finale;
mod v2_5_compiler;
mod v2_5_lift_finale;
mod v2_5_lift_locals;
mod v2_5_lint;
mod v2_6_compiler;
mod v2_6_lift_finale;
mod v2_6_lint;
mod v2_compiler;
mod v2_fmt_and_decompile;
mod v2_lift_panic_and_discard;
mod v2_lift_pilot;
mod v2_lift_pr_h_idioms;
mod v2_lift_rust_polish;
mod v2_lift_wave2;
mod v2_lift_wave3;
mod v2_lint;
mod v2_lint_wave2;
mod v2_lint_wave3;
mod v2_lint_wave4;
mod v2_lint_wave5;
mod v2_lint_wave6;
mod v2_panic_and_discard;
mod v2_pr_g;
mod v2_pr_h;
mod v2_pr_h_context;
mod v2_pr_h_cursor;
mod v2_pr_h_inline;
mod v2_pr_h_review;
mod v2_pr_i4_and_std_utils;
mod v2_r3_fixes;
mod v2_rust_syntax;
mod v2_typed_std;
