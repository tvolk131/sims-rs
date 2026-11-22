# ADR: `sims-game` Wiring for Special `Dialog` Kinds `5..=15`, Native `cWinCamera`, 300 ms Hover Roof Fade & Temporal UI Animations

- **Date:** 2026-11-22
- **Branch:** `game/wire-special-panels-camera-and-animations`
- **Scope:** `crates/sims-game/` and `docs/decisions/` (strictly disjoint from `sims-vm`, `sims-render`, `sims-ui`, `sims-antics`)

## Context

PR #373 landed full VM support for all eleven special `Dialog` kinds (`5..=15`), including single-player `Lot::asking_special_panel()` / `Lot::answer_special_panel(...)`, inline reference books (`SpellBook` 12 and `CookBook` 15 via `InlineSpecialPanel` / `wait_for_books`), and co-op lockstep `CoopBatchReport::opened_special_panel` / `sims_vm::coop::special_panel_answer_command(...)`.

PR #371 landed native Complete Collection 1.91 UI & render building blocks:
- `sims_ui::camera::CameraPanel` (`cWinCamera` subpanel with `PanelBack.BMP`, `CameraPatch.bmp` cluster overlay, all 8 native buttons including `CamViewAlbum` `1105`, Quick Tips help balloons, and `STR# 140` strings loaded from `SIMS_INSTALL`).
- `sims_render::neighborhood::HoverFadeState` (300 ms per-lot roof cross-fade with simultaneous multi-lot fade-in/out).
- Temporal UI animation helpers: `BalloonFadeState` (250 ms), `DialogFadeState` + `draw_with_state_alpha`, and `QueueSlideState` (500 ms + `Queue::at_clickable`).

This decision records how `crates/sims-game/` wires both PR #373 and PR #371 end-to-end while preserving 100% bit-exact static headless/ratchet rendering at `clock_us == 0` and ~0% idle CPU when animations settle.

## Decisions

1. **Uniform Special Panel Modal Wiring (`Dialog` kinds `5..=15`, Single-Player + Co-op):**
   - `Game::sync_special_panel()` polls `Lot::asking_special_panel()` alongside ordinary blocking dialogs (`Lot::asking()`), pauses `advance_while` and `edge_scroll`, and opens the matching native modal:
     - **Destination Lot Pickers (5 Downtown, 7 Vacation, 8 Old Town, 11 Studio Town, 13 Magic Town):** Open native 9-slice `ShopPickerDialog` populated from real neighborhood/installed lot metadata (`*Desc.iff` `STR# 2000+id` + `StreetNames.iff` + `Neighborhood::zoning`), with zero hardcoded game prose. Answering with a lot replays the tick cleanly via `Lot::answer_special_panel(...)` and queues `pending_travel_lot` so `NeighborhoodShell::handle_lot_outcome` executes `depart_to_destination` cleanly (never halting or stranding; ineligible departures stay safely on the lot with a native notice).
     - **Interactive Pickers (6 WardrobeShop, 9 PetPicker, 10 CallNeighbor, 14 CharmTransform):** Populate `ShopPickerDialog` from real install skins/catalog entries or live `World::neighbors` records (with real names, portraits, and daily/lifetime relationships for Kind 10) and deliver `SpecialPanelResponse::Selection` / `Neighbor` without client-side currency mutation.
     - **Inline Reference Books (12 SpellBook, 15 CookBook):** Open native `OptionsDialog::SpecialBook` (`cTSWinMsgBox`, `FrameStyle::Popup`) displaying real caller inventory tokens resolved via `CTSS`/`OBJD` labels until dismissed (`SpecialPanelResponse::Dismissed`).
   - **Co-op Lockstep:** `Game::apply_coop_batch_side_effects` handles `CoopBatchReport::opened_special_panel` and auto-closes local modals when another peer resolves the panel first. `Game::answer_special_panel` submits `sims_vm::coop::special_panel_answer_command(dialog_id, &response)` through lockstep.

2. **Native `cWinCamera` Subpanel (`sims_ui::camera::CameraPanel`):**
   - Replaced PR #370's interim Camera sub-strip in `Hud` with `sims_ui::camera::CameraPanel`, loading `sims_ui::camera::{RESOURCES, HELP_RESOURCES}` and `CameraStrings` (`STR# 140`) in `HudStatic::load`.
   - Draws `CameraPatch.bmp` (`2035`) over the cluster at `[0, height - 183] + [165, 83]` and all 8 native buttons strictly right of `CLUSTER[0]` (`x >= 220`).
   - Wires size (`Small`/`Medium`/`Large`) and quality (`Low`/`Medium`/`High`) toggles in place, `CamViewAlbum` (`1105`) to open Photo Album while closing `CameraPanel`, Quick Tips help balloons (`1109`/`1110`), house-viewport click + auxiliary `CamSnap` button strictly right of `CamViewAlbum` to close `CameraPanel` before clean PNG capture, and clean dismissal on Escape / Back / mode switches while preserving `F10`.

3. **300 ms Neighborhood Hover Roof Cross-Fade (`HoverFadeState`):**
   - `NeighborhoodShell` owns `HoverFadeState` driven by presentation clock `clock_us / 1_000`.
   - When `clock_us == 0`, `RenderOptions` uses instantaneous `hover_roof_fade = 1.0` for bit-exact static/headless renders.
   - When `clock_us > 0`, `HoverFadeState::apply_to` populates `lot_roof_fades` for simultaneous multi-lot fades and mouse-leave fade-back-in. `has_active_fades()` keeps `RedrawPacer` at display cadence only while transitions are running, emits one final landed frame on settlement, prunes finished fade-out ramps, and stops waking at idle.

4. **Temporal UI Animations (`BalloonFadeState`, `DialogFadeState`, `QueueSlideState`):**
   - Wired in `Hud::draw_for_lot_at(..., clock_us)` and `NeighborhoodShell` (for shell dialogs):
     - `BalloonFadeState`: 250 ms fade-in/fade-out via `balloon::draw_with_alpha` (or `slide_offset_y` when Interface Effects is disabled).
     - `DialogFadeState`: 200 ms fade-in via `dialog::draw_with_state_alpha` on both Live/VM dialogs and Shell dialogs.
     - `QueueSlideState`: 500 ms action-queue icon slide-in from pie-menu click position (`[click - 16]`) and slot-shift interpolation, with `Queue::at_clickable` ignoring clicks on mid-slide icons (`0x4cb406`).
   - All three snap instantaneously when `clock_us == 0` so static pixel ratchets remain 100% bit-exact.

## Consequences

- Clicking the Goth Magic Hole opens the native Magic Town destination picker with zero crashes or unsupported traps in both single-player and co-op.
- Live Camera Mode renders authentic Complete Collection 1.91 `cWinCamera` chrome with `CameraPatch.bmp`, `STR# 140` tooltips/Quick Tips, and zero overlap with the Live clock.
- Neighborhood hover roofs and Live HUD balloons, dialogs, and action-queue icons animate smoothly in interactive desktop play while keeping static `clock_us == 0` ratchets bit-exact and idle CPU near zero.
