# Photo Album / Family Storybook UI & Environmental Lot Ambience Mixer

- **Date:** 2026-11-21
- **Branch:** `ui/photo-album-and-ambience`
- **Crates:** `sims-ui` (`album.rs`), `sims-audio` (`ambience.rs`)

## Summary

Adds two disjoint Track 6 library components ready for `sims-game` and REDB integration:

1. **Photo Album / Family Storybook Modal (`crates/sims-ui/src/album.rs`):**
   - Native Complete Collection 1.91 modal viewer/editor (`cWinPhotoAlbum`) rendered with `PopupInfoTiles.bmp` (`3008`), `WinBtn.bmp` (`17`), `HotspotPopupTiler.TGA` (`5107`), `CamViewAlbum.bmp` (`1105`), and `cTSWinTextEdit2` dark-navy text fields (`RGB(0, 0, 0x52)` quantized to RGB565, 500 ms blinking caret).
   - Pure view-model API (`PhotoId`, `AlbumEntry`, `AlbumSnapshot`, `AlbumCommand`) with zero simulation or REDB coupling.
   - Supports household photo browsing (First/Prev/Next/Last, clickable scrollable thumbnail strip, keyboard navigation), adding blank pages, triggering camera snapshots (`AlbumCommand::CaptureSnapshot`), reordering entries (`MoveEntry`), two-step inline delete confirmation (`DeleteEntry`), and single-line title + word-wrapped multiline caption editing with exact byte-range caret tracking and auto-commit on navigation/close.
   - Runtime localization via `AlbumStrings::load_from_install` reading `GameData/UIText.iff` (`STR# 140` `cammodestrs`, `STR# 141` `scrapbookstrs`, `STR# 142` `DefaultDialogButtons`, `STR# 154` `MiscStrings`), embedding zero verbatim game prose in source or tests.
   - Press-bound pointer safety: pointer-up verifies the release lands on the same enabled `ControlId` (and matching `PhotoId` for entry-bound actions) captured on pointer-down.

2. **Environmental Lot & Expansion Ambience Mixer (`crates/sims-audio/src/ambience.rs`):**
   - Extends `sims_audio::ambience` alongside the existing 1.91 `cFreshPlayer` (`Fresh`) FSC score player without altering any existing public API or behavior.
   - `AmbienceCatalog::from_sound_files` indexes Complete Collection's environmental background loops across `SoundData/SoundData.far` (`Outdoors`, `MTOutdoors`) and `ExpansionShared/Sound/Sound.far` (`Beach`, `Park`, campfire loops) into `(AmbienceZone, TimeOfDay)` buckets (`Residential`, `Downtown`, `VacationBeach`, `VacationForest`, `VacationSnow`, `StudioTown`, `MagicTown` × `Day`/`Night`), preferring continuous `*_loop.xa` beds and excluding `*I.XA` indoor-duplicate stems so indoor muffling is controlled continuously.
   - `AmbienceMixer<S: TrackSource>` streams and seamlessly loops XA/WAV/MP3 tracks via `StreamDeck` with zero silent-sample gaps across loop boundaries, smooth fixed-point crossfades on day↔night (`TimeOfDay::from_hour` matching `HOUR_COLUMNS`) and zone transitions, Complete Collection's 11-step `VOLUME_STEPS` FX/Ambience volume curve, and smoothly ramped camera `ZoomLevel` (`Far`/`Medium`/`Near`) × indoor/outdoor (`outdoor_percent`, `0..=100` compatible with `ambience::view`) attenuation scaling.

## Trade-offs & Follow-Ups

- **Reusing `crates/sims-audio/src/ambience.rs`:** Because `ambience.rs` already owns the 1.91 FSC `Fresh` player, `HOUR_COLUMNS`, `SCORES`, and `view(...)`, placing `AmbienceCatalog` and `AmbienceMixer` in the same module keeps all ambient audio primitives together and allows hosts to feed `ambience::view(...)` directly into `AmbienceMixer::set_outdoor_percent(...)`.
- **Host Wiring (deferred to gameplay integration PR):**
  - Bind `PhotoAlbumDialog` to REDB household photo storage, F-key / Camera panel `CamViewAlbum` button, and live-lot framebuffer capture (`CaptureSnapshot`).
  - Mix `AmbienceMixer::mix_into_stereo` alongside `Player`, `ModeMusicPlayer`, and `RadioPlayer` in `sims-game/src/sound.rs`, driving `set_zone`, `set_hour`, `set_zoom`, `set_outdoor_percent`, and FX `VOLUME_STEPS`.
