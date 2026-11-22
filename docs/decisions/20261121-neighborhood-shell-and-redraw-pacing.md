# Neighborhood Shell Loop, Streaming Mode Music, and Hybrid Display-Refresh Redraw Pacing

## Context

With the three Neighborhood pillars landed on `main` — `sims-render::neighborhood` (#341), `sims-sim::neighborhood_store` + `sims-vm::lifecycle` (#343), and `sims-ui::{neighborhood, family_bin}` + `sims-audio::music` (#344) — `sims-game` previously only opened a single `<house file>` directly into `Game`, left Options → Neighborhood returning a placeholder status note, did not stream Neighborhood/Buy/Build MP3 mode music through `sound::Mixer`, and requested a full `softbuffer::present()` on every simulation wake (resulting in 200+ FPS single-core CPU burn during hands-off Ultra speed).

## Decisions

### 1. Decoupled Headless Screen State Machine (`crates/sims-game/src/neighborhood.rs`)
- Added `NeighborhoodShell` encapsulating the `Neighborhood ↔ FamilyBin ↔ Lot` screen transitions while keeping `Game::open` untouched and edits to `crates/sims-game/src/game.rs` minimal (`Outcome::neighborhood`, `Game::open_from_store`, `Game::save_to_neighborhood`, `Game::is_actively_interacting`, `Game::render_parts_mut`).
- CLI launches without a `<house file>` (or with `--neighborhood`) open `NeighborhoodShell` on slot 1 (`UserData`), auto-importing `UserData`..`UserData8` into the `--saves` REDB store on demand without ever modifying `SIMS_INSTALL`. Explicit `<house file>` launches and `--png` stills remain 100% backwards-compatible.
- Because `redb::Database` takes an exclusive file lock, `NeighborhoodShell` owns the single `NeighborhoodStore` handle for the session and routes F5/F9 and Options → Save/Neighborhood through `sims_vm::lifecycle::save_and_leave_lot` / `enter_lot`.
- Lot hover cards enrich `NeighborhoodMap` hits with `StreetNames.iff` (`STR# 2000`/`2001`), `NeighborhoodDesc.iff` (`STR# 2002`/`2003`), `Neighborhood.iff` `FAMs` surnames, `sims_ui::household::Faces` member portraits, and live REDB valuations (`lot_price`, `family_net_worth`). All confirmation dialogs (Move-In, Evict, Bulldoze, Delete Family, Quit) load localized text dynamically from `GameData/UIText.iff` (`STR# 128`, `131`, `132`, `142`, `153`, `161`).
- On `leave_lot(true)` and `execute_bulldoze`, map thumbnails (`MapLot.normal` / `roofs_off`) and REDB occupancy/Family Bin state refresh immediately.

### 2. Streaming Mode Music Wiring (`crates/sims-game/src/sound.rs`)
- Embedded `sims_audio::music::ModeMusicPlayer<SoundFiles>` inside `sound::Mixer`, driven by a new `Input::MusicMode(MusicMode)` event, synchronized with `Input::MusicVolume` (`kind 38`), `Input::Active` focus pause/resume, and `SoundEvent::StopAll`.
- `sound::Feed::update` tracks live-lot mode transitions (`Live`, `Buy`, `Build`, `Options`, plus Makin' Magic `MagicTownBuy`/`MagicTownBuild` on lots `90..=98`), while `NeighborhoodShell` queues `Neighborhood` / sub-neighborhood / `FamilyBin` transitions on screen switches.

### 3. Hybrid Display-Refresh / 60 FPS Redraw Cap (`crates/sims-game/src/main.rs`)
- Added `RedrawPacer` to decouple simulation stepping from visual presentation:
  - `App::tick()` continues stepping owed ticks at full Normal/Fast/Ultra speed inside `SIM_BUDGET` (8 ms).
  - Physical display refresh rate is queried via `window.current_monitor().and_then(|m| m.refresh_rate_millihertz())` (fallback 60 Hz, refreshed on `resumed`, `ScaleFactorChanged`, `Moved`, and `Focused(true)`).
  - During active user interaction (pointer move/click/wheel/key within a 250 ms hold, right-drag pan, edge-scrolling, slider/dialog drags, Buy/Build placement cursors, or modal dialogs), visual redraws run up to the physical monitor refresh rate (e.g. 120 Hz on ProMotion, 60 Hz on 60 Hz displays).
  - During passive simulation-only updates (hands-off Fast/Ultra ticks, neighborhood water animation), visual redraws + `softbuffer::present()` are capped at 60 FPS (`PASSIVE_FPS_INTERVAL = 16.67 ms`, or the monitor's own interval on sub-60 Hz displays), eliminating 200+ FPS busy-spinning while keeping input silky-smooth.
  - `ControlFlow::WaitUntil` schedules the next wake at `min(now + WAKE, next_redraw_deadline)` so deferred frames present on time.

## Verification
- Unit tests in `main.rs`, `neighborhood.rs`, and `sound.rs` cover `RedrawPacer` (60 Hz, 120 Hz, 30 Hz, fallback, monitor switch, interactive vs passive transitions), CLI launch-mode compatibility, `StreetNames` formatting, and vacant thumbnails.
- End-to-end headless integration tests in `crates/sims-game/tests/neighborhood_shell.rs` verify Neighborhood open → lot hover (`StreetNames.iff` + portraits) → enter Goth lot → tick → Options → Neighborhood confirmation (Cancel + Save & Leave) → REDB persistence → neighborhood `1↔2` switching → Family Bin Move-In → Evict → Bulldoze, plus streaming Mode Music + `MusicVolume` slider wiring.
