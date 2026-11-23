# 2026-11-22: Build Mode OBJD Word 69 Mapping, Buy Mode 100 px Strip, Roof/Walls 4-Way Radio Group, and Graphical Destination Maps

## Context

Playtesting on Complete Collection 1.91 surfaced four high-impact UI/gameplay discrepancies across Build Mode, Buy Mode, camera view toggles, and travel lot pickers:

1. **Build Mode catalog categories shifted off by two:** `sims_ui::build::tool_for_build_mode_type` mapped `OBJD` word 69 (`build_mode_type`, offset `0x8a`) using `3..=9`, causing clicking **Doors** to show stairs, **Windows** to show plants, **Stairs** to show fireplaces, and **Fireplace** to show pool ladders/diving boards.
2. **Buy Mode 50 px empty blue overhang:** `sims_ui::hud::layout_with_options_lot` selected `4918` (`kTallSubpanel`, `TallSubPanel.TGA`, `150 px`) whenever `controls.mode == Mode::Buy`, while `sims_ui::buy::layout_with_sell` laid out all Buy Mode icons, subcategories, catalog slots, and sell prompts inside the standard `100 px` `STRIP`, leaving a 50 px opaque blue rectangle protruding into the lot view.
3. **Roof & Walls view buttons not mutually exclusive:** `CameraState::roof` was treated as an independent toggle alongside `CameraState::walls`, allowing both `id::ROOF` and `id::WALLS_DOWN` (or `CUTAWAY` / `WALLS_UP`) to light up cyan simultaneously and drawing a roof over lowered walls.
4. **Placeholder `ShopPickerDialog` used for destination lot pickers:** Jumping into the **Magic Hole** (`Dialog` kind 13, `MagicTownLotPicker`) or taking a Cab/Shuttle to **Downtown** (`5`), **Vacation Island** (`7`), **Old Town** (`8`), or **Studio Town** (`11`) opened `ShopPickerDialog` with `"Buy"`, disabled `"Try On"`, `"§0"`, empty thumbnail boxes, and misleading string-table titles instead of the real interactive destination map.

## Verification Against `Sims.exe` 1.91 & Complete Collection Assets

1. **OBJD word 69 (`0x8a`) vs `0x4cbb08..0x4cbb58`:**
   - In `Sims.exe` `0x4cbaf9..0x4cbb5a`, wall/floor entries (`[ecx + 0x24] == 1`) dispatch through the jump table at `0x4cbb5d` pushing shifted internal table IDs `3..=9` to `0x487c28`; non-wall/floor `OBJD` objects (`[ecx + 0x24] != 1`) compare directly against `[ecx + 0x38]` (`OBJD` word 69) at `0x4cbb35`.
   - Scanning every `.far` and `.iff` in Complete Collection confirms every non-zero `OBJD` word 69 value is in `1..=7`:
     - `1`: Doors (49 items)
     - `2`: Windows (48 items)
     - `3`: Stairs & Teleporters (18 items)
     - `4`: Plants — trees, shrubs, flowers (63 items)
     - `5`: Fireplaces (6 items)
     - `6`: Columns, Arches, Object Fences under Wall Tool (46 items)
     - `7`: Pool objects — Pool Ladder, Diving Board, Pool Slide under Water Tool (3 items)
2. **Buy Mode strip background (`0x509af5..0x509f55`, `0x4cfde1`):**
   - `cWinSubPanel` loads `kUniversalCPBkg` (`4910`, `PanelBack.bmp`, `804×100`) into `[esi + 0x104]` at `0x509af5`.
   - `kTallSubpanel` (`4918`, `TallSubPanel.TGA`, `600×150`) is loaded only when `0x4cfde1` returns true (expansion community-lot Live Mode score bar, `byte [gLot + 0x64] ∈ {0xf, 0x11, 0x12, 0x13, 0x14}`) and passed exclusively to `cWinLive` (`[esi + 0xe4]`, `0x509dc1`).
   - `cWinCatalog` (Buy Mode, `[esi + 0xe0]`, `0x504d37`) unconditionally receives `[esi + 0x104]` (`4910`, `PanelBack.bmp`) with height `0x64` (`100 px`, `0x509e6f..0x509e9d`), identical to `cWinArch` (Build) and `cWinOptions` (Options).
3. **4-way Roof/Walls radio group & `Home`/`End` (`0x4cef10`, `0x53a34d`, `0x53ad4f..0x53adb5`, `0x4d135b..0x4d13eb`):**
   - `Sims.exe` stores a single 4-state wall/roof display enum at `[ui + 0x48]` (`0x5ecdca`): `1` = `Walls::Up`, `2` = `Walls::Down`, `4` = `Walls::Cutaway`, `8` = `Roof`.
   - `cWinCPanel` (`0x53ad4f..0x53adb5`) lights strictly one of `{id::ROOF, id::WALLS_UP, id::CUTAWAY, id::WALLS_DOWN}` matching `[ui + 0x48]`.
   - Clicking `kLevelRoof` (`2025`, `0x53a34d`) calls `0x4cef10(8, 1)`, enabling roof rendering (`0x4b1459(1, 0)`) with full walls. Clicking any of the 3 wall buttons calls `0x4cef10(1|4|2, 1)`, turning roof rendering off (`0x4b1459(0, 0)`).
   - `OnKeyDown` `Home` (`0x4d135b`) steps `Down (2) → Cutaway (4) → Up (1)` (playing `UI_error` at `Up`), and `End` (`0x4d1393`) steps `Up (1) → Cutaway (4) → Down (2)` (playing `UI_error` at `Down`); when `[ui + 0x48] == 8` (`Roof`), neither `Home` nor `End` matches `1`/`2`/`4` and Roof view stays active without playing an error. `PageUp`/`PageDown` (`0x4cee08` -> `0x4a80c7`) change floor level without mutating `[ui + 0x48]`.

## Decisions & Changes

1. **`crates/sims-ui/src/build.rs`:** Corrected `tool_for_build_mode_type` to map `1 => Doors`, `2 => Windows`, `3 => Stairs`, `4 => Plants`, `5 => Fireplace`, `6 => Wall`, `7 => Water` (and made it public for direct verification alongside real-install catalog sanity tests). Also fixed the three misaligned `OBJD` byte offsets (`OBJD_GRAPHICS = 0x08`, `OBJD_TTAB = 0x0e`, `OBJD_CTSS = 0x52`, matching `buy.rs` and `Sims.exe` `0x4cd149`/`0x457a36`) so all Complete Collection Build Mode catalog objects load their authentic localized `CTSS` names and descriptions instead of falling back to raw internal chunk labels with empty descriptions.
2. **`crates/sims-ui/src/balloon.rs`:** Generalized `slot_vertical`'s widescreen slot-0 layout branch from `frame_width == 1024` to `frame_width >= 1024` so help balloons stay bit-exact at `800×600` and `1024×768` without a 50 px jump at `1024 → 1025` / `1280` / `1600` / `1920`.
3. **`crates/sims-ui/src/hud.rs`:** Removed `let tall = controls.mode == Mode::Buy;` so Buy Mode uses standard `100 px` `id::STRIP` (`4910`, `PanelBack.bmp`) with zero 50 px overhang while keeping `id::BUY_TRIM` (`4804`) and `id::BACK_PATCH` (`2036`) aligned. Updated `{id::ROOF, id::WALLS_UP, id::CUTAWAY, id::WALLS_DOWN}` selection flags to strict 4-way mutual exclusion (`!camera.roof && camera.walls == ...`).
4. **`crates/sims-game/src/game.rs`:**
   - `set_roof_view(true)` raises walls to `Walls::Up` alongside `roof_view = true`.
   - `UiCamera::Roof` selects Roof mode as a radio button (`set_roof_view(true)`).
   - `Game::apply(Move::Walls(_))` clears `roof_view` and returns `true` when exiting Roof view even if `camera.walls` was already `Walls::Up`.
   - `Game::step_walls` preserves `Sims.exe` `0x4d135b..0x4d13eb` behavior when `roof_view` is active.
5. **`crates/sims-game/src/destination_picker.rs`, `crates/sims-ui/src/neighborhood.rs`, `crates/sims-game/src/game.rs`, `crates/sims-game/src/neighborhood.rs`, `crates/sims-game/src/main.rs`:**
   - Added `DestinationMapPicker` backed by the real `sims_render::neighborhood::NeighborhoodMap` (supporting all 5 destination lot-picker `Dialog` kinds `5, 7, 8, 11, 13`: Downtown, Vacation Island, Old Town, Studio Town, Magic Town) with native backdrops, animated water waves, 300 ms `HoverFadeState` roof cross-fades, interactive `TravelChrome` (`TravelBarMode::Visit`) filter bar & Return button (or native Old Town return banner), and native `draw_hovered_lot_popup` House Info cards.
   - Visitable destination lots (`21..=30`, `40..=48`, Old Town community `zoning == 2`, `81..=89`, Magic Town `93..=98`) are clickable and deliver `SpecialPanelResponse::Lot(Some(lot_id))` (triggering real `NeighborhoodShell` travel transitions and co-op lockstep broadcast); non-visitable lots (such as Magic Town residential lots `90..=92`) refuse with `UI_error`; Return button and `Esc` deliver `SpecialPanelResponse::Lot(None)` cleanly.
   - Removed the fake `build_destination_lot_picker` `ShopPickerDialog` path entirely.
   - Wired `destination_picker_open` into `KeyboardFocus`, `Escape`/`Enter` dispatch, `wheel` suppression, and `NeighborhoodShell` slot propagation.
6. **Structural Invariant & Regression Tests:**
   - `crates/sims-ui/tests/invariants.rs`: resize continuity (`1024 → 1025` max widget delta `<= 2 px` across HUD/Live/Buy/Build/Options/Camera + `balloon::slot_vertical` across `800`/`1024`/`1025`/`1280`/`1440`/`1600`/`1920`), 4-way view & mode radio mutual exclusion + 100 px Buy strip invariants across `800×600`, `1024×768`, `1025×768`, `1280×720`, `1440×900`, `1600×900`, and real-install `SIMS_INSTALL` Build Mode catalog asset-sanity assertions for all 7 `OBJD` word 69 buckets plus `>= 95%` localized `CTSS` description coverage.
   - `crates/sims-game/tests/playtest_regressions.rs`: end-to-end Goth Roof/Walls 4-way radio transitions, zero-overhang Buy Mode pixel check, and full `NeighborhoodShell` Magic Hole / Downtown graphical destination map E2E travel & cancel tests.

## Verification

- Verified `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, full `cargo test --release --workspace`, `8/8 ratchet: ok`, `4/4 ui_ratchet`, Goth 43,200-tick hash `df6eb544...`, and `python3 scripts/check_repository.py --history HEAD` (`0 violations`).
- Visually inspected rendered PNG screenshots in `/tmp/build-buy-roof-shots/` covering Buy Mode (By Function, By Room, subcategory & popup card), Build Mode (Doors, Windows, Stairs, Fireplace, Water), Roof View, and the Magic Town Hole graphical destination map.
