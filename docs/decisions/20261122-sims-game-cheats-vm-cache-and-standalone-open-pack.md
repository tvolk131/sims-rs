# ADR: Native Live Cheats, Warm `VmInstallCache`, Native CAF Surnames, SHA-256 Autosave Fast Path, and Dual-Mode Standalone `open-pack/` Boot

- **Date:** 2026-11-22
- **Status:** Accepted
- **Scope:** `crates/sims-game`, `crates/sims-ui`, `crates/sims-vm`

## Context

Following PR #383 (`6c347669`, fast Save Manager listing, 3D Try-On preview, paginated Spellbook/Cookbook, and Neighborhood secondary actions), PR #384 (`f906d5a4`, `sims-antics` formal verifier and 20-object clean-room standalone `open-pack/`), and PR #385 (`0b5e1fa1`, `sims-vm` native live cheats, `VmInstallCache`, 600-tick deep sweep ports, REDB v4 `StoredNeighborhood.family_surnames`, and `AutosaveStore` SHA-256 validator fast paths), `sims-game` still had four runtime seams pointing at pre-PR #385 workarounds, and could not boot without a commercial `SIMS_INSTALL` directory:

1. **Live cheats** (`move_objects`, `rosebud` / `klapaucius`, `set_hour`) either kept a UI-only flag (`"VM placement override pending"`) or round-tripped through `Lot::restore`, which wiped Buy/Build undo history and cleared `neighborhood_origin`.
2. **Warm lot entry** cached render/UI/people assets in `InstallAssets`, but still rebuilt the entire base VM `Program`, animations, avatar registry, surfaces, and icon frames on every `NeighborhoodShell::enter_lot`.
3. **CAF family surnames** still wrote a legacy `<saves>.caf-names.tsv` sidecar file instead of relying on native REDB v4 `StoredNeighborhood.family_surnames`.
4. **Autosave validation** re-hashed payloads inside `validate_save_payload_cached` even when `AutosaveStore` had already computed the SHA-256 digest.
5. **Zero-commercial-asset standalone boot:** Anyone cloning `sims-rs` without an EA/Maxis Complete Collection installation could not launch `sims-game` or render a playable lot.

## Decision

### Part 1: Wire PR #385 Primitives into `sims-game`

1. **Native Live Cheats (`crates/sims-game/src/game.rs`):**
   - Removed the UI-only `cheat_move_objects: bool` shadow field; `Game::cheat_move_objects()` now reads `self.lot.move_objects()` directly.
   - `move_objects on/off` calls `self.lot.set_move_objects(on)` and reports honest active feedback (`"move_objects on"` / `"move_objects off"`), blocking cleanly while co-op is active or starting.
   - `rosebud` / `klapaucius` (including `;!` multipliers) call `self.lot.add_funds(total_funds_delta)` directly, and `set_hour <0..23>` calls `self.lot.set_hour(hour)` directly. Neither mutates via `Lot::restore`, so Buy/Build undo stacks and `neighborhood_origin` remain intact and co-op refusal errors surface cleanly.

2. **Warm Lot-Entry `VmInstallCache` (`crates/sims-game/src/game.rs`, `crates/sims-game/src/neighborhood.rs`):**
   - `InstallAssets` now loads and caches `Arc<sims_vm::lifecycle::VmInstallCache>` once per installation in parallel with render/UI/people assets (`InstallAssets::vm_cache()`).
   - `Game::open_from_store_cached` calls `sims_vm::lifecycle::enter_lot_with_cache` whenever `InstallAssets` is present, and `NeighborhoodShell::save_cheat_mutated_lot_to_store` also reuses `vm_cache`.
   - **Measured `--release` latency on Complete Collection `House05.iff` (Apple Silicon, 5-run median):**
     - `perf_part4_track1_measurements_and_parity` (comparing pre-cache `InstallAssets`-only baseline on `0b5e1fa1` vs cached `InstallAssets + VmInstallCache`):
       - `first(lot5)`: **726.38 ms → 423.86 ms** (**1.71× faster**)
       - `warm(lot5)`: **394.14 ms → 169.49 ms** (**2.33× faster**)
       - `warm(lot7)`: **391.82 ms → 167.89 ms** (**2.33× faster**)
     - `complete_collection_warm_enter_lot_vm_cache_parity_and_speedup` (5-run median uncached `open_from_store_cached(..., None)` vs warm `NeighborhoodShell::enter_lot(5)`):
       - **555.78 ms → 183.62 ms** (**3.03× faster**), with bit-identical `World::hash()`.

3. **Native REDB v4 CAF Surnames (`crates/sims-game/src/neighborhood.rs`):**
   - Removed `save_persisted_caf_name` and all `.caf-names.tsv` write paths.
   - `load_family_names_with_caf` overlays native REDB v4 `StoredNeighborhood.family_surnames` over any existing pre-v4 `<saves>.caf-names.tsv` (kept strictly read-only as a migration fallback), and `NeighborhoodShell::refresh_from_store` syncs `self.family_names` from the refreshed `StoredNeighborhood`.

4. **`AutosaveStore` SHA-256 Validator Fast Path (`crates/sims-game/src/autosave_worker.rs`, `crates/sims-game/src/neighborhood.rs`):**
   - Added `validate_save_payload_cached_with_sha256(kind, bytes, sha256)` so the LRU validation cache keys directly off the digest already computed by `AutosaveStore`.
   - Wired both `AutosaveWorker` commit paths to `store.commit_job_validated_with_sha256` and both `NeighborhoodShell` load paths (`load_autosave_slot_into_lot`, `restore_current_lot`) to `store.load_save_validated_with_sha256`.

### Part 2: Dual-Mode Standalone `open-pack/` Boot

1. **CLI & Automatic Fallback (`crates/sims-game/src/main.rs`, `crates/sims-game/src/open_pack.rs`):**
   - Added `--open-pack [dir]` (discovering `open-pack/` from the executable directory, current working directory, parent directories, or `CARGO_MANIFEST_DIR`).
   - When neither `<install>` nor `SIMS_INSTALL` is provided, `sims-game` automatically falls back to standalone `open-pack/` mode (including headless `--png <out.png> [--ticks N] [--size WxH]`).
   - All existing Complete Collection invocations (`<install>`, `SIMS_INSTALL`, `--neighborhood`, `--png`, `--play`, `--strict`, etc.) remain 100% unchanged and bit-exact.

2. **Clean-Room Procedural UI Art (`crates/sims-ui/src/art.rs`, `crates/sims-game/src/hud.rs`):**
   - Added `sims_ui::art::Art::clean_room_procedural()`, generating an original 5×7 clean-room bitmap font scaled for all 8 `FONT_SIZES` (`0x20..=0x7E` plus `§`) and beveled procedural dark-navy chrome bitmaps for HUD panels, buttons, pie menus, action queue icons, Buy/Build catalogs, and cheat console dialogs.
   - Added `HudStatic::clean_room_procedural()` and `Hud::open_open_pack(...)` using clean-room procedural Sim portraits (`procedural_sim_portrait`).

3. **Clean-Room Staging & Boot (`crates/sims-game/src/open_pack.rs`, `crates/sims-vm/src/boot.rs`, `crates/sims-game/src/game.rs`, `crates/sims-game/src/sound.rs`):**
   - `stage_open_pack` compiles the clean-room `open-pack/` Antics scripts (`sims-antics`), object manifests, procedural walls/floors, cottage lot, starter Sims, global tuning tables (`FCNS 1/2/4/5/6`, `STR# 502/504/505`), lot zoning (`LotZoning.iff`), and looping indicator stubs into a temporary clean-room virtual directory cleaned on drop via `std::fs::remove_dir_all`.
   - Added a strictly additive opt-in `Lot::load_open_pack` entry point in `crates/sims-vm/src/boot.rs` that registers minimal clean-room `Skeleton` entries (`"open_sim"`) for clean-room starter Sims without modifying any Complete Collection boot path.
   - When `Game::open_pack_mode` is `true`, `Game::render` draws clean-room 2D isometric Sim sprites and a bright green plumb bob over the selected Sim via `sims_render::depth::Depth` + `Canvas::draw` (skipped entirely in Complete Collection mode).
   - Added `Mixer::open_pack` (`OpenPackAudioBank`) in `crates/sims-game/src/sound.rs` to decode and linearly resample clean-room 16-bit PCM WAV clips to 22,050 Hz and mix them on `SoundEvent::Play`, leaving `Mixer::open` untouched for Complete Collection.

## Consequences

- Complete Collection behavior stays 100% bit-exact (Goth 43,200-tick hash `df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a`, `8/8 ratchet: ok`, `4/4 ui_ratchet`).
- Warm lot switches in Complete Collection are **2.33×–3.03× faster** with zero behavioral divergence.
- Live cheats preserve Buy/Build undo history and `neighborhood_origin`, and `move_objects on/off` genuinely overrides VM placement collision.
- Zero new third-party dependencies (`sims-antics` is an in-workspace crate) and `unsafe_code = "forbid"` remains enforced workspace-wide.
- Anyone can run `cargo run --release -p sims-game` (or `cargo test -p sims-game --test cheats_cache_and_open_pack` in CI with `SIMS_INSTALL` unset) and play the clean-room cottage with interactive pie menus, Buy Mode, live cheats, camera controls, and synthesized WAV sound effects.
