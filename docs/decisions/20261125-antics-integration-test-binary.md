# One `sims-antics` integration test binary, with features gated per module

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/tests/` and the `[[test]]` entries of
  `crates/sims-antics/Cargo.toml`. No `src/` file changes.
- **Track:** Track 7
- **Builds on:** [`20261125-integration-test-binary-consolidation.md`](20261125-integration-test-binary-consolidation.md)

## Context

`sims-antics` had 30 `tests/*.rs` files, each built and linked as its own
binary. Five of them needed an optional feature and were declared in
`Cargo.toml` as `[[test]]` entries with `required-features`:

- `install-catalog`: `cli_lift` and `structured_equiv`;
- `vm-replay`: `live_equiv`, `live_equiv_corpus` and `antics_test_runner`.

## Decision 1: one `tests/integration/` binary, as in the core crates

Every file moved with `git mv` to `tests/integration/<name>.rs`, and
`tests/integration/main.rs` declares one `mod <name>;` per file. A test that
was `<name>` in binary `<target>` is now `<target>::<name>` in binary
`integration`. To select one module:
`cargo test -p sims-antics --test integration <target>::`.

The `tests/test_runner/` fixtures stay where they were. They have no
`main.rs`, so Cargo does not build them as a target, and the test that reads
them builds the path from `CARGO_MANIFEST_DIR`.

## Decision 2: `#[cfg(feature)]` on the module replaces `required-features`

`required-features` applies to a whole target. Kept on the single binary, it
would drop every test from a `--no-default-features` run, where the 25
ungated binaries used to run. So the five `[[test]]` entries are removed, and
`main.rs` gates the same five modules with `#[cfg(feature = "...")]`. An
integration test sees the package's own features, so the gate is the same
one Cargo applied before.

## Consequences

- **No test lost.** Every old name maps to exactly one new name: 626 tests
  listed with default features, 487 with `--no-default-features`, in release
  and debug. The PR has the size and time measurements.
- **Modules share a process.** Tests from different modules now run in
  parallel in one process. No test sets environment variables or changes
  the working directory. Every scratch directory name starts with a
  per-module prefix (some also add a per-module counter), so names that
  included only the process id stay unique.
- **Install sweeps take turns.** Five modules each have one
  `SIMS_INSTALL` test: `behavioral_equiv`, `cli_lift`, `install_roundtrip`,
  `live_equiv_corpus` and `structured_equiv`. A sweep keeps up to 64
  archives open. As separate binaries they ran one after another. In
  parallel, two of them hit a 128-descriptor limit on a synthetic
  100-archive install. Each now holds `common::install_sweep_lock()` for
  its whole run, which keeps the old one-at-a-time order. The lock
  recovers from poisoning, so a failed sweep does not fail the next.
- **Old paths in docs.** Older ADRs keep the `tests/<name>.rs` paths they
  named at the time. No doc, script or workflow used a `--test` selector for
  a `sims-antics` target.

## What would reopen this

- A test that needs process-global state (environment, working directory,
  a global allocator), which forces its module back into its own binary.
- A feature gate that Cargo cannot express as module `cfg`, such as a
  dev-dependency that only some modules may link.
