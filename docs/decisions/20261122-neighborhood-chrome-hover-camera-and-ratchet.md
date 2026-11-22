# Neighborhood Chrome Pixel Parity, Hover Roof Cross-Fade, Native Camera Subpanel, and UI Pixel Ratchets

Date: 2026-11-22
Status: Accepted

## Context

Live playtesting of `main` (`b4973e65`) against Complete Collection 1.91 (`Sims.exe`) surfaced three visible visual/animation differences plus missing temporal UI animation helpers and automated UI pixel ratchets:

1. The Neighborhood top-bar page number (`1..=8`) rendered a `Current.bmp` plate that protruded above the lavender top bar instead of matching 1.91's centered white digit.
2. Hovering a residential lot on the Neighborhood map drew a synthetic yellow edge outline by default and popped the roof off instantaneously instead of smoothly fading the roof out over 300 ms with no default outline.
3. The Photo Album button (`CamViewAlbum`, resource `1105`) had no native Complete Collection Camera Mode subpanel (`cWinCamera`) to live in.
4. Temporal UI transitions (help-balloon 250 ms fade/slide, dialog global/corner alpha compositing, and action-queue 500 ms icon slide-in) needed pure-state animation helpers verified against `Sims.exe`.
5. `sims-ui` and `sims-render` lacked automated pixel-comparison ratchets against real 1.91 Neighborhood, Family Bin, and Create-a-Family/Create-a-Sim captures.

## Decisions

### 1. Neighborhood Top-Bar Number Plate (`crates/sims-ui/src/neighborhood.rs`)
- Disassembly of `cWinNeighborhood::Init` (`0x556e17..0x556e8c`) and `cWinNeighborhood::DoDraw` (`0x55ce70..0x55d120`) shows that `Current.bmp` (`5206`, `23×30`) is loaded solely to size `[0x6f5890..0x6f589c]` for centering the white `variablesans_12` (`0x556daa`) neighborhood digit and is **never blitted** onto the bar.
- Removed the `Current.bmp` blit and aligned `badge_rect` to `ox + 124, y = 3` (`23×30` sizing) with `0x55d062..0x55d102` centering. Against `01-nb-clean.png`, the entire `800×52` top bar now matches Complete Collection 1.91 **41,600 / 41,600 pixels (100.00% bit-exact in 5-6-5)**.
- Fixed Create-a-Family (`crates/sims-ui/src/caf.rs`) synthetic overlays that painted over baked-in `DsgnFamBkg.bmp` and `CreateACharBack.bmp` wells/LEDs, raising CAF Family parity from `85.92%` to `96.18%` and CAF Create-a-Sim chrome parity from `74.60%` to `95.33%`.

### 2. Neighborhood Hover Roof Cross-Fade & Native Highlight Rules (`crates/sims-render/src/neighborhood.rs`)
- Verified in `cWinLot` (`0x54ea7f..0x54f560`) and Track 2's 1.91 captures (`02-nb-hover-goth.png`, `03-nb-hover-lot6.png`) that plain residential hover **never** calls `0x54f28f` (yellow highlight outline). Changed `RenderOptions::default().highlight_hovered` to `false` and exported the native mode-highlight colors (`VACANT_HIGHLIGHT_RGB = [0x00, 0xc8, 0x00]` at `0x54f535`, `OCCUPIED_HIGHLIGHT_RGB = [0xf2, 0x00, 0x00]` at `0x54f54a`, alongside `DEFAULT_HIGHLIGHT_RGB` and `COMMUNITY_FILTER_HIGHLIGHT_RGB`).
- Added `RenderOptions::hover_roof_fade: f32` (default `1.0` for backwards compatibility), `RenderOptions::lot_roof_fades: BTreeMap<u16, f32>`, `HOVER_FADE_DURATION_MS = 300` (`0x12c` at `0x54eae8`, `0x54eb28`), `fade_to_alpha`, and pure-state `HoverFadeState` implementing `cWinLot`'s 300 ms `cRZLinearRamp` (`[esi + 0x21c]`) including continuous mid-fade reversal (`1.0 - cur`) and simultaneous multi-lot fade-in/fade-out.
- Added `blit_crossfade_to_canvas` using the original's 5-bit 5-6-5 blend LUT (`canvas::blend565`, `0x6e5eac`) with exact backdrop substitution on `COLOR_KEY_565` mismatches (`0x54ef8b..0x54f120`) so overhanging roofs dissolve cleanly into the map backdrop, while keeping `Canvas::id_at` and `hit_test_with_options` in 100% agreement inside the clipped `800×600` lot rectangle (`map_clip`; unclipped overhang pixels outside `map_clip` remain hit-testable via `hit_test_with_options` per `0x5dc83c` / `0x5e4071`).

### 3. Native Complete Collection Camera Mode Subpanel (`crates/sims-ui/src/camera.rs`)
- Disassembled `cWinCamera` (`0x50333e..0x5043e0`, button table `0x66a310`) and verified against Complete Collection 1.91 `UIGraphics` and `GameData/UIText.iff STR# 140` (`cammodestrs`).
- `CamBack.bmp` (`1100`) is absent in 1.91; `cWinCamera` draws `PanelBack.BMP` (`4910`), vertical dividers `CameraToothpkLeft.bmp` (`4914` at `(133, 6)`) and `CameraToothpkRight.bmp` (`4915` at `(206, 6)`), cluster overlay `CameraPatch.bmp` (`2035` at `(165, 83)`, `0x53a1a9`), the eight 4-cell buttons (`1101..=1108`, including `CamViewAlbum` `1105` at `(226, 35)`), and help balloons (`1109`, `1110`).
- Implemented `CameraPanel`, `CameraLayout`, `CameraState`, `CameraStrings::load` (zero hardcoded prose), `CameraButton`, `CameraHelp`, `CameraCommand`, and `CameraPanel::draw_cluster_patch` as a self-contained `sims-ui` module ready for Track 1 to wire into `sims-game` without changing `sims_ui::command::Mode`.

### 4. Temporal UI Animation Helpers (`crates/sims-ui/src/{balloon,dialog,queue}.rs`)
- **Help Balloon (`balloon.rs`, `0x50f882..0x50fbdc`)**: Added `FADE_DURATION_MS = 250`, `BalloonFadeState` (250 ms linear fade + continuous mid-close reopen + Interface Effects off vertical slide), and `draw_with_alpha`.
- **Dialog Fade & 9-Slice Alpha (`dialog.rs`, `0x524603..0x524645`, `0x5e8c93..0x5e8e10`)**: Added `draw_nine_slice_alpha`, `DialogFadeState`, and `draw_with_state_alpha`.
- **Action Queue Slide (`queue.rs`, `0x4cb190..0x4cb406`)**: Added `SLIDE_DURATION_MS = 500`, `QueueSlideState`, and `Queue::at_clickable` (ignoring clicks on mid-slide icons, matching `0x4cb406`).

### 5. Automated UI & Neighborhood Hover Pixel Ratchets
- Added `crates/sims-ui/tests/ui_ratchet.rs` covering Neighborhood top bar (100.00% clean, >=99.5% Move-In, >=99.0% Bulldoze), Family Bin chrome (>=97.5%, measured 98.00%), CAF Family (>=95.5%, measured 96.18%), CAF Create-a-Sim chrome (>=94.5%, measured 95.33%), and live-install `CameraPanel` loading/rendering.
- Added `neighborhood_hover_cutaway_and_crossfade_match_191_oracle` to `crates/sims-render/tests/neighborhood_map.rs` verifying 100.00% clean lot accuracy, zero yellow outline pixels + >=98.5% Goth Lot 5 cutaway match against `02-nb-hover-goth.png`, and intermediate 300 ms 5-6-5 cross-fade frames.
- All oracle PNG ratchets gate cleanly on `SIMS_INSTALL` plus optional `SIMS_NB_ORACLE` / `SIMS_CAF_ORACLE` directories so CI skips them when private oracle captures are absent.
