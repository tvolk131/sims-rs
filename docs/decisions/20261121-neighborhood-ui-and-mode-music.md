# Neighborhood Screen Chrome, Family Bin & Mode Music (1.91)

- **Date:** 2026-11-21
- **Scope:** `crates/sims-ui/src/neighborhood.rs`, `crates/sims-ui/src/family_bin.rs`, `crates/sims-audio/src/music.rs`
- **Reference Binary:** The Sims Complete Collection 1.91 (`Sims.exe` disassembly + live `800×600` captures)

## 1. Neighborhood Top Control Bar (`crates/sims-ui/src/neighborhood.rs`)

Disassembled from `cWinCommunityNeighborhood` (`0x556531..0x55660a`, `0x556b80..0x557881`, `0x55b2ab..0x55d120`) and verified against live `800×600` captures (`01-nb-clean.png`, `11-nb-movein-mode.png`, `17b-bulldoze-hover-lot4.png`):

- **Artwork (`UIGraphics/Res_Nbhd.h`):**
  - Top bar banner: `kNghBarBkg` (`5420`, `NghUI/banner_neighborhood.bmp`, `800×52`).
  - Logo: `kNghSimsLogoEn` (`5421`, `NghUI/TheSimsLogo.bmp`, `70×52`, 1 cell) at `(5, 0)`.
  - Neighborhood switcher (`1..=8`): `kNghPrevious` (`5204`, `26×52`) at `(106, 0)`, `kNghCurrent` (`5206`, `23×30`) at `(125, 3)` with `variablesans_12` white digit centered (`0x55d062..0x55d102`), `kNghNext` (`5205`, `26×52`) at `(141, 0)`.
  - 9 action & expansion buttons (4-cell `53×52` strips spaced at 50 px intervals):
    - `SelectFamily` (`5200`, `200, 0`), `ImportFamily` (`5207`, `250, 0`), `Bulldoze` (`5201`, `300, 0`), `Rezone` (`5215`, `350, 0`), `Downtown` (`5208`, `400, 0`), `VacationIsland` (`5213`, `450, 0`), `StudioTown` (`5216`, `500, 0`), `MagicTown` (`5221`, `550, 0`), `Quit` (`5202`, `600, 0`).
  - `Web` (`5306`, `39×52`) at `(762, 0)` and `Credits` (`5315`, `58×58`) at `(195, 540)` on the bottom-left island.
- **Hit Targets & Narrow Windows:**
  - Adjacent `53 px` circular badges share a `3 px` colorkey glow margin at `50 px` stride; hit rects trim that margin to `50 px` so hit targets never overlap while drawing matches 1.91 pixel-for-pixel.
  - Below `800 px` width, controls pack evenly into the available bar width with strictly disjoint hit boxes.
- **Strings:** Loaded dynamically from `GameData/UIText.iff` (`STR# 151` tooltips, `STR# 225` missing-expansion notices). No game prose is embedded in source or tests.

## 2. Lot Hover / House Info Popup Card (`crates/sims-ui/src/neighborhood.rs`)

Disassembled from `0x54f756..0x551060` and verified against `02-nb-hover-goth.png`, `04-nb-hover-empty.png`, `11-nb-movein-mode.png`, and `17b-bulldoze-hover-lot4.png`:

- **Frame:** 9-slice alpha tiler `kHotspotPopupTiler` (`5107`, `Nbhd/HotspotPopupTiler.TGA`, `111×111` divided `3×3` into `37×37` tiles at `0x5509cf..0x550a20`).
- **Typography & Sizing:** `variablesans_10` in `[195, 205, 205]` (`0x6f2a94`), `25 px` padding on all sides (`0x5507ea`), text width clamped to `[150, 320]` px (`0x5507a2..0x5507bc`), widened `3/2×` up to 2 times whenever `total_h >= (total_w * 2) / 3` (`0x55084c..0x550871`).
- **Portraits:** Occupied households append a `10 px` gap (`0x55082d`) and 1 or 2 rows of `25×25` portraits (`0x550b0e`).
- **Content (`UIText.iff STR# 134`):** Covers occupied residential (Normal / Bulldoze / Rezone), vacant residential (Normal / Move-In affordable & unaffordable for both houses and empty lots / Bulldoze demolish vs empty), Magic Town Creepy Hollow Simoleons + MagiCoins checks, Tutorial lots, and Community lots.
- **Clamping:** Popup placement clamps inside `[0, frame_w)` × `[BAR_HEIGHT, frame_h)` so it never occludes top-bar buttons.

## 3. Family Bin / Select a Family (`crates/sims-ui/src/family_bin.rs`)

Disassembled from `cWinPickFamily` (`0x563284..0x5642c2`), `PickFamilyList` (`0x543918..0x544220`), and `PickFamilyItem` (`0x5429a6..0x543918`), verified against `09-nb-family-bin.png`, `10-family-selected.png`, and `09-family-bin-with-alice.png`:

- **Backgrounds:** `kPickFamilyBkg` (`5041`, `Nbhd/PickBkg.bmp`, `800×600`) for standard neighborhoods and `kPickFamilyBkgMagic` (`5043`, `Nbhd/PickBkgMagic.bmp`, `800×600`) for Magic Town.
- **Family List:** Positioned at `(41, 76)` (`0x563299`), showing `6` rows (`0x543c63`) of `690×68` (`kPickFamilyItemBkg = 5019`, `690×204` split vertically into 3 states: normal=0, hover=1, selected=2). Empty slots below the populated families still render state 0 frames.
- **Row Contents:** Surname in `variablesans_12` at `(20, 0)`, up to 8 member portraits (`45×45`, stride `48 px`) starting at `(110, 14)`, Net Worth (`$20,000`), Friends, and optional MagiCoins in `variablesans_10` at `y = 21..25` with drop shadow `[0, 0, 82]` (`0x6f2aa4`) and state colors `[195, 205, 205]` / `[255, 255, 255]` / `[0, 255, 255]`.
- **Scrollbar:** `kPickScrollbar` (`5018`, `Nbhd/ScrollBar.BMP`) appears automatically whenever `families.len() > visible_rows`, supporting arrows, track paging, thumb dragging, and wheel scrolling.
- **Action Buttons:** In 1.91, household eviction is performed via the Neighborhood screen's **Evict or Bulldoze** tool (`kNghBulldoze`), while the Family Bin holds unassigned households with 4 bottom buttons (`0x5632b3..0x5632f9`): **Cancel** (`kNghDefaultBtn = 5301`, centered at `x = 200, y = 529`), **Create New Family** (`5103`, `(322, 530)`), **Delete Family** (`5105`, `(414, 530)`), and **Move In Family** (`5106`, `(506, 530)`). Delete and Move-In are disabled (cell 3) when no household is selected.
- **Strings:** Loaded from `GameData/UIText.iff` `STR# 128`. Title font loads `Fonts/variablesans_16.ffn` (falling back to 14/12 if absent).

## 4. Mode Music Playback (`crates/sims-audio/src/music.rs`)

Disassembled from `kSetMusicMode` (`0x573fe2` → `0x575170`), station registration (`0x5718b2..0x571a12`), and `SoundData/SimsSound.hit` (`MainMusicLoop` `0xc6`, `SetMainAndHitListKillFirst` `0x11e`, `SetMainMusicSongAndHitList` `0x13a`, `FadeAndKillTrackSlow` `0x191`):

- **Why a dedicated streaming module alongside `player::Player`:**
  - `player::Player` already executes the HIT VM and decodes patches through `SoundFiles`, but decodes an entire MP3 synchronously into memory (~45 MB PCM per song).
  - `ModeMusicPlayer` streams MP3 packets incrementally via `symphonia_bundle_mp3::{MpaReader, MpaDecoder}` (plus WAV for `Music/Modes/Load/loadloop.wav`), resampling on the fly to `output_rate` (default `22,050 Hz` stereo) with tiny memory footprint and zero decode stalls.
- **1.91 Semantics Preserved:**
  - Maps all 19 `kSetMusicMode` values (`0..=18`), selecting `Music/Modes/NhoodUS` (US English) vs `Music/Modes/Nhood` (non-US) per `0x571905`, plus `Buy`, `Build`, `Load`, and all expansion mode directories.
  - Deterministic Fisher-Yates shuffle per mode preventing immediate track repeats across reshuffles when `tracks.len() >= 2`.
  - Exact transition rules: Buy/Build → Live stops immediately; Neighborhood/expansion → Live runs `FadeAndKillTrackSlow` (`fade_speed = 3` per 20 ms tick ≈ 6.84 s); FamilyBin → Neighborhood, Options, and MagicTownBuild ↔ MagicTownBuy preserve the active track; switching between two active stations applies a smooth crossfade.
  - Music volume slider (`0..=10`) scales output using the exact 1.91 `VOLUME_STEPS` table (`0x67156c`, `kind 38` / `kGroupMusic` group 2), yielding exact silence at step `0`.
