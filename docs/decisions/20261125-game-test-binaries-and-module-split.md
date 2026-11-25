# Integration Tests as One Binary per Crate; `game.rs` and `neighborhood.rs` Split

*Decided 2026-11-25.*

**Branch:** `t3/game-test-consolidate-and-module-split`
**Track:** Track 1

## Context

The four presentation crates built 56 integration-test binaries:

| Crate | Test binaries |
|---|---|
| `sims-game` | 23 |
| `sims-ui` | 20 |
| `sims-render` | 8 |
| `sims-people` | 5 |

Each binary statically links the crate and its whole dependency tree, so every change relinked 56 large executables. Together they took about 690 MiB in `target/release/deps`.

`sims-game`'s two shells were also single files of 15,281 lines (`game.rs`) and 9,271 lines (`neighborhood.rs`). Almost all of each was one `impl` block.

Neither change is meant to change behaviour.

## Decision

### Test binaries: 56 → 6

- **One binary per crate.** Each crate's `tests/<name>.rs` moved with `git mv` to `tests/integration/<name>.rs`, as module `<name>` under a new `tests/integration/main.rs`. Cargo builds that as the test target `integration`.
- **Two `sims-game` binaries stay separate:**
  - `ratchet.rs`, which uses its own harness (`harness = false`);
  - `ui_oracle_tour.rs`.
- **`sims-ui`'s `tests/common`** moved to `tests/integration/common`.
  - Its users now write `use crate::common;` in place of `mod common;`.
  - `examples/hud.rs` follows the new `#[path]`.
- **A `common` module in each crate shares helpers whose bodies were the same.**
  - `sims-game`: `install_root` (the variant that checks `is_dir`) and the Goth house `Options`.
  - `sims-ui`: the `is_dir` and detected installs, and `intersects`.
  - `sims-render`: the detected install.
  - `sims-people`: `hex`.

  A module that named one of these differently imports it under its old name (for example `use crate::common::install_root as install;`). No test body changed.
- **Helpers that differ stay in their modules.** These include install helpers with their own skip message or without the `is_dir` check, and fixtures that load different houses. Merging them would change what a test does when `SIMS_INSTALL` is unset or wrong.
- **No `OnceLock` fixture is shared.** Each test that opens a house, a `Game` or a shell mutates what it opened: it ticks the world, draws into its assets, or writes saves. An immutable fixture worth sharing would be at most `Install::detect`, which is cheap. Nothing mutable is shared between tests.
- **Running in one process is safe:**
  - No test sets environment variables or changes the working directory.
  - Temporary directories carry the test's module-specific prefix plus the process id, so no two modules use the same path.
  - The library's process-wide state is limited to content-keyed caches (the neighborhood render cache, the highlight LUT) and id counters.
- **Test names gain the module.** `cargo test -p sims-game --test goth foo` becomes `cargo test -p sims-game --test integration goth::foo`. Earlier ADRs keep the old paths as history.

### `game.rs` and `neighborhood.rs`: module directories

- **Mechanical split.** `game.rs` became `game/mod.rs`, and `neighborhood.rs` became `neighborhood/mod.rs`.
  - The `impl Game` and `impl NeighborhoodShell` blocks were cut at method boundaries into contiguous ranges.
  - Each range moved verbatim, with its doc comments and section banners, into an `impl` block in a child module that starts with `use super::*;`.
- **Types, free functions and other `impl`s stay in `mod.rs`.** Their private fields and helpers stay visible to every child.
- **Unit tests moved.** Each `#[cfg(test)] mod tests` moved to `tests.rs` unchanged, apart from its indentation.
- **The only edit inside moved code:** a moved method with no visibility became `pub(super)`. That is 114 methods in `game/` and 63 in `neighborhood/`. `pub(super)` reaches exactly what private reached before: the parent module and its children. Methods that were `pub` or `pub(crate)` keep that.
- **Module map:**
  - `game/`:
    - `opening`;
    - `shell_state` (animations, cheats, camera strip, travel requests, debug, roof, voices, sound);
    - `dialogs`;
    - `ticking`;
    - `drawing`;
    - `pointer`;
    - `commands` (save and restore, panel commands, hotkeys);
    - `buy_mode`;
    - `build_mode`;
    - `shop_dialog`;
    - `book_dialog`;
    - `try_on`;
    - `special_panels`;
    - `coop_shell`;
    - `tests`.
  - `neighborhood/`:
    - `opening`;
    - `shell_state`;
    - `drawing`;
    - `pointer`;
    - `keys`;
    - `lot_saves`;
    - `lots_and_families`;
    - `commands`;
    - `rezone_and_import`;
    - `photo_album`;
    - `travel_shell`;
    - `coop_shell`;
    - `tests`.
- **Names avoid clashes.** A child module never takes a name the crate already uses (`coop`, `sound`, `hud`, `view`, ...).
- **Splitting further is left for later.** `game/mod.rs` keeps the `Game` struct, the other types and the free helpers (2,383 lines). `neighborhood/mod.rs` keeps 1,745 lines. Splitting those further means choosing which helpers belong to which child, and that is no longer mechanical.

## Verification

**The split is a rearrangement.** A script puts the child modules back in order into `mod.rs`, drops the added `pub(super)` and module headers, and compares the result with the original file. The comparison ignores whitespace and trailing commas.
- `game` (387,449 characters) and `neighborhood` (237,483) are identical.
- All 546 and 339 string literals are byte-identical.

**No test is lost.** Every test listed before is listed after, with only its module prefix added (`<target>::` → `integration::<target>::`). There are 0 missing and 0 extra.

Measurements on the same machine and target directory:

| | Before (`c40bcb8`) | After |
|---|---|---|
| Integration test binaries | 56 | 6 |
| Their size in `target/release/deps` | 691.9 MiB | 99.6 MiB |
| All test executables of the four crates (with unit tests) | 61, 717.0 MiB | 11, 124.8 MiB |
| Tests listed (`-- --list`, four crates) | 789 | 789 |
| Rebuild after touching the four crates (`--no-run`) | 71.1 s | 26.3 s |
| `cargo test --release` wall clock (built) | 174.9 s | 105.1–131.7 s |

The test results are the same before and after: 783 passed, 0 failed, 5 ignored. The `ratchet` shares are unchanged on all 8 scenes.

Wall-clock times were measured on a shared, heavily loaded machine (load average above 100), so treat them as rough. The run-time range covers one run after the test consolidation (105.1 s) and one after the module split (131.7 s), which changed no test.
