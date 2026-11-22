# ADR: Interactive UX Polish — `--debug` Banner Gate, Camera Sub-strip, Play-Mode Default & Neighborhood Hover/Pan Polish

- **Date:** 2026-11-22
- **Status:** Accepted
- **Scope:** `crates/sims-game/` (`hud.rs`, `game.rs`, `neighborhood.rs`, `main.rs`, tests)

## Context

Playtesting `main` (`b4973e65`) against Complete Collection 1.91 surfaced four immediate UX differences in `sims-game`:

1. **Overlapping `CamViewAlbum` (`1105`) button in Live Mode:** `Hud::compute_camera_album_button_rect()` drew `CamViewAlbum` unconditionally at `photo.rect.x - 33`, covering the `AM 8:41` clock readout and crowding the Camera icon in the Live Mode cluster.
2. **Always-on diagnostic top status banner:** Routine diagnostic notes (e.g., `Queued "Open" for person 21.`) drew a maroon `[96, 24, 24]` bar (or blue neighborhood bar) across the top of the viewport during normal gameplay.
3. **Strict halt default on interactive launches:** Launching the interactive desktop window without `--play` halted the entire lot on the first unported long-tail SimAntics script trap instead of resetting the faulting Sim/object and continuing.
4. **Neighborhood hover/animation & portrait centring feel:**
   - Idle residential/destination browsing enabled `RenderOptions::highlight_hovered = true`, drawing a synthetic yellow outline box on hover instead of 1.91's roof-cutaway-only idle hover.
   - Sweeping the cursor across neighborhood lots popped the House Info card instantaneously on frame 0 instead of waiting for a brief steady hover.
   - Re-clicking / double-clicking a Sim portrait snapped the camera origin instantaneously instead of gliding smoothly toward the Sim (`0x4c9a27`).

## Decisions

### 1. Remove Floating Live-Mode `CamViewAlbum` & Add Bottom-Strip Camera Sub-strip
- Removed `compute_camera_album_button_rect()` and the floating `CamViewAlbum` overlay completely so the Live Mode HUD cluster (`0..220`) stays bit-exact with 1.91.
- Clicking the cluster Camera button (`Command::Camera(Photo)`) now toggles a native-styled Camera sub-strip inside the bottom panel strip area (`x >= hud::CLUSTER[0] = 220`, zero overlap with the clock, speed controls, or Live cluster) offering **Take Snapshot** and **Photo Album (`F10`)**.
- Switching modes, toggling the panel, opening a modal dialog, pressing `Escape`, or clicking the Camera button again closes the sub-strip. `F10` continues to open the Photo Album directly.
- `App::photo()` re-renders after closing the sub-strip so captured PNG snapshots do not include the Camera sub-strip.

### 2. Gate Top Diagnostic/Status Banners Behind `--debug` + Native Notice Dialogs
- Added a `--debug` CLI flag (default `false`) propagated into `Game::set_debug` and `NeighborhoodShell::set_debug`.
- When `debug == false`, the maroon top status band in `Game::draw` / CAF and the Neighborhood top status strip are suppressed; programmatic `Game::note()` and `NeighborhoodShell::status_note()` getters remain unchanged so headless tests keep working.
- When `debug == true`, diagnostic banners render and newly recorded diagnostic notes log to stderr.
- Genuine interactive user-facing errors (save/restore errors, travel unavailable, Photo Album unavailable, co-op startup/restage errors, standalone-only Save Manager/Neighborhood clicks, and `--strict` lot halts) surface via a native Sims 1 modal OK dialog (`cTSWinMsgBox` via `OptionsDialog::Notice` / `ShellDialogKind::Notice`).

### 3. Default Interactive Desktop Play to Graceful Play Mode (`--strict` Opt-in)
- `Options::play` remains `false` for programmatic/headless `Game::open` callers and `--png` still captures, keeping every strict oracle and replay test unchanged.
- In `main.rs::parse_launch_args`, interactive windowed launches (`png.is_none()`) default to `play: true` (resetting faulting objects/Sims and logging caught errors to stderr without freezing the lot).
- Added `--strict` to opt into strict halt-on-error mode, kept `--play` accepted for backwards compatibility, and rejected `--play` combined with `--strict`.

### 4. Neighborhood Hover, Water Ripples & Smooth Portrait Camera Pan
- **Hover highlighting:** `NeighborhoodShell::should_highlight_hovered_lot()` sets `RenderOptions::highlight_hovered = false` during plain idle residential and destination browsing (preserving immediate `hover_roofs_off: true` cutaway), and `true` only during Move-In, Bulldoze, or active Community Filter modes (also wiring `active_filter` and `filter_frame` into `RenderOptions`).
- **Water ripples:** `NeighborhoodShell::tick_shell()` advances `water_frame` smoothly at `WATER_STEP_US = 200_000` (5 Hz), checking `map.waves.iter().any(Option::is_some)` (and active filter animations) to schedule redraws.
- **Hover-card debounce:** Hovering a lot records `hovered_lot_id` immediately (for roof cutaway) while delaying `nbhd_state.hovered_lot` House Info card population by `HOVER_CARD_DELAY_US = 275_000` (275 ms) of steady hover (suppressed while modals/dialogs are open) and clearing immediately on moving off or leaving the lot.
- **Smooth camera pan:** In `Game`, re-clicking / double-clicking a Sim's portrait interpolates `camera.origin` smoothly with a cubic ease-out curve over `CAMERA_PAN_DURATION_US = 350_000` (350 ms) whenever interactive presentation time is running (`clock_us > 0` or `set_smooth_camera_pan(true)`), running even while paused at display-refresh cadence (`is_actively_interacting`) and cancelling immediately on manual scroll/zoom/rotate/resize/restore. At `clock_us == 0` (static ratchet and headless replay frames), portrait centring snaps instantaneously so zero oracle/ratchet frames change.

## Consequences
- All changes are strictly scoped to `crates/sims-game/` and this ADR, with zero overlap with `sims-vm`, `sims-render`, `sims-ui`, `ratchet.txt`, or `docs/roadmap.md`.
- Existing strict replay tests, ratchet frames, and programmatic `note()`/`status_note()` assertions remain green.
