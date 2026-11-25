# One integration test binary per core crate

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/{sims-sim,sims-audio,sims-content,sims-formats,sims-net}/tests/`. No `src/` file changes.
- **Track:** Track 4

## Context

Cargo builds every `tests/*.rs` file as its own binary. Each binary links the
crate and all its dependencies again. Five core crates had 42 such binaries:

| Crate | Before | After |
| --- | --- | --- |
| `sims-sim` | 13 | 1 |
| `sims-audio` | 12 | 1 |
| `sims-content` | 9 | 1 |
| `sims-formats` | 6 | 1 |
| `sims-net` | 2 | 1 |

They also repeated setup work. With `SIMS_INSTALL` set:

- `sims-sim` ran `Index::build` over the real install in eight places;
- `sims-content` ran it in four;
- `sims-audio` scanned the install's sound files or loaded its modules in
  27 places.

## Decision 1: each crate gets one `tests/integration/` binary

Every old target moved with `git mv` to `tests/integration/<name>.rs`.
`tests/integration/main.rs` declares one `mod <name>;` per old target.

- **Test names.** A test that was `<name>` in binary `<target>` is now
  `<target>::<name>` in binary `integration`. To select one module:
  `cargo test -p <crate> --test integration <target>::`.
- **Shared helpers.** `sims-content`'s `tests/common/mod.rs` became the
  module `integration/common.rs`.
- **`sims-formats`.** `avatar_codecs` keeps its four submodules in
  `integration/avatar_codecs/`. They resolve without the old `#[path]`
  attributes.

## Decision 2: shared helpers are read-only `OnceLock`s and one path counter

Each crate's `common.rs` holds the helpers more than one module repeated:

- **`sims-sim` and `sims-content`.** `installed_index(&Install) -> Index`
  builds the real install's index once per process and returns a clone.
- **`sims-audio`.** Three helpers:
  - `install(skipping)` reads `SIMS_INSTALL`;
  - `sound_files(root)` scans the sound files once;
  - `modules(root)` loads the modules once.
- **`sims-sim`.** `TempDb` and `TempDir` replace seven per-module temporary
  path guards (`TempDb` ×4, `Temp`, `TempPath`, `TempDir`), and an
  inline temporary directory.

These helpers are safe to share for three reasons:

- **Nothing is shared mutably.** Each `OnceLock` is written once and then
  only read. Callers get their own clone; `Index`, `SoundFiles` and `Module`
  are `Clone`. A test that mounts characters mounts them on its clone.
- **No test changes the install.** Every test writes only to its own
  temporary folders, so separate builds would all be equal.
- **One install per run.** Each helper asserts the root it is asked for is
  the one it built from.

The `AtomicU64` in `sims-sim`'s and `sims-content`'s `common.rs` only names
temporary paths. The modules used to run in separate processes, so a name of
label plus process id was unique. In one process, two modules with the same
label could collide. The counter holds no test state.

Helpers with the same name but different bodies were left in their modules.
Only identical code was shared, and no `sims-formats` or `sims-net` helper
was repeated.

## Consequences

- **Fewer links.** 42 test binaries became 5 with no test lost. Every
  before name maps to exactly one after name (584 tests listed before and
  after). Measurements are in the PR.
- **Shared build cost.** `SIMS_INSTALL` runs pay once per crate for the index
  and the sound files.
- **One behaviour change.** In `sims-audio`, `radio`, `music` and `ambience`
  used to skip silently when `SIMS_INSTALL` was set but was not a
  directory. Now they fail, like the crate's other install tests. Unset still
  skips.
- **Stale selectors elsewhere.** Docs that name an old target need the new
  selector:
  - `docs/repository-workflow.md` ("Avatar-codec test selectors"):
    `--test avatar_codecs` becomes `--test integration avatar_codecs::`.
    An `--exact` name needs the full path, e.g.
    `avatar_codecs::avatar::skeleton_hierarchy_and_truncations`; the old
    `avatar::…` name would select zero tests and still exit 0;
  - file paths in `docs/legacy-mining-audit.md`.

  Older ADRs keep their paths as written at the time.
- **Leftover directory.** The empty `crates/sims-content/tests/common/`
  directory is left on disk (untracked). Git does not track it.

## What would reopen this

- A test that needs to change the install, or global state, which forces
  its module back into its own process.
- A crate whose single binary's link time grows past the sum of the old
  ones' incremental rebuilds in day-to-day use.
