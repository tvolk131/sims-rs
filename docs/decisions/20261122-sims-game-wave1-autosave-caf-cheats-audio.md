# `sims-game` Integration Wave 1: Background Autosave, CAF, Cheat Console, Pie Anim & Streaming Audio

- **Date:** 2026-11-22
- **Status:** Accepted
- **Scope:** `crates/sims-game/` (strictly disjoint from library crates and `tests/ratchet.txt`)

## Context

The decoupled library crates now expose Complete Collection 1.91 backends for:
- Crash-safe REDB manual & rotating autosaves + startup migrations (`sims-sim`: `AutosaveStore`, `NeighborhoodStore::migrate_all`),
- Save Manager & classic Cheat Console UIs (`sims-ui`: `SaveManagerDialog`, `CheatConsole`),
- Create-a-Family UI, 3D turntable & 6-portrait renderer, and atomic REDB household commit/delete (`sims-ui::caf`, `sims-people::caf_preview`, `sims-vm::caf`),
- 400 ms pie-menu pop-in, submenu glide, and head direction ease-out (`sims-people::pie_head::PieAnimState`), and
- Streaming stereo/TV radio and environmental lot ambience (`sims-audio`: `RadioPlayer`, `AmbienceMixer`).

Integration Wave 1 wires all five pillars end-to-end inside `crates/sims-game/` without modifying any library crate or shifting any baseline screenshot ratchet pixel.

## Decisions

1. **Background CoW Save Worker (`crates/sims-game/src/autosave_worker.rs`):**
   - `NeighborhoodShell::open` runs `NeighborhoodStore::migrate_all()` non-destructively, recording any migration warning in status feedback rather than aborting or overwriting saves.
   - `AutosaveWorker` owns a cloned `sims_sim::AutosaveStore` on a dedicated `"sims-autosave-worker"` thread. Callers enqueue either raw `SaveJob` (`SavePayload::World(Box<World>)` with `expected_instance_id` fencing) or boxed `Saved` lot snapshots whose `Saved::encode()` + SHA-256 + REDB commit execute off the UI thread.
   - Manual saves (`F5` / Options → Save), periodic midnight rotating autosaves (sim clock wrap to `00:00`), and Save & Leave autosaves route through the worker, with synchronous `flush()` on lot exit and window close (plus clean `Drop` join).
   - `SaveManagerDialog` opens from Options → **Saves** (`Ctrl/Cmd+L`, or non-destructive corrupt-save fallback on F9 / lot entry) and supports `LoadSlot`, `FallbackToNewestIntact`, `DeleteSlot`, and `Cancel`.
   - Because `Lot::restore` clears `neighborhood_origin`, `NeighborhoodShell::save_current_lot` falls back to `StoredNeighborhood::write_back_world` + `save_lot_and_neighborhood_checked` after restoring a save slot or applying a world-mutating cheat.

2. **End-to-End Create-a-Family (`crates/sims-game/src/caf_screen.rs`):**
   - `CafSession` bridges `sims_ui::caf::CafController`, `sims_people::caf_preview` (live 3D turntable sweep/drag preview + 45×45 roster portraits + all six `BMP_ 2002..=2007` portraits), and `sims_vm::caf::commit_family_with_context`.
   - Family Bin **Create New Family** transitions to `ScreenKind::CreateFamily`; committing updates `family_names`, refreshes REDB state, and returns to the Family Bin with the new household pre-selected for immediate Move-In.
   - Family Bin **Delete Family** delegates to `sims_vm::caf::delete_bin_family` for tombstone-shielded deletion.

3. **Classic Cheat Console, 400 ms Pie Animations & `edge_scroll` Hover Refresh (`game.rs`, `hud.rs`, `menu.rs`, `people.rs`, `main.rs`):**
   - `Ctrl+Shift+C` / `Cmd+Shift+C` toggles `sims_ui::cheat_console::CheatConsole`. World-mutating commands (`rosebud` / `klapaucius` +$1,000 per chained command, `set_hour`) batch cleanly through lossless `Saved` postcard round-tripping while preserving UI/camera/console state; `move_objects`, `map_edit`, `water_tool`, `autonomy`, `sim_speed`, `interests`, `prepare_lot`, and `help` apply directly to `Game`/`Lot`.
   - `Menu` holds `Option<PieAnimState>` enabled whenever `clock_us > 0` or `Game::set_pie_animations(true)`, preserving timeless steady-state unit/ratchet tests bit-for-bit while driving smooth 400 ms transitions at display refresh rate via `RedrawPacer`.
   - `Game::edge_scroll` refreshes `hover_build` / `hover_buy` immediately after panning so Build/Buy placement previews track the scrolled terrain.

4. **Streaming Radio & Environmental Ambience (`crates/sims-game/src/sound.rs`):**
   - `Mixer` mixes `RadioPlayer<SoundFiles>` and `AmbienceMixer<SoundFiles>` alongside `Player` and `ModeMusicPlayer`, tracking window focus (`active`) and live gameplay pause (`paused_by_game`) independently so focus-regain while paused and `StopAll` silence invariants remain intact.
