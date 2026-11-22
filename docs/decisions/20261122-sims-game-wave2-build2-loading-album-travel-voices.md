# ADR: `sims-game` Integration Wave 2 — Build Mode Part 2, Loading Screen, Photo Album & Persistent Thumbnails, Travel Destinations Shell, and Simlish Voices

- **Date:** 2026-11-22
- **Status:** Accepted
- **Scope:** `crates/sims-game/` only (zero edits to library crates or `tests/ratchet.txt`)

## Context

With all Wave 2 library pillars merged on `origin/main` (`#350` Travel VM, `#353` Travel UI Chrome/Dialog, `#356` Build Mode Part 2 Render Previews, `#357` Photo Album UI, `#358` Build Mode Part 2 UI/VM, `#361` Shop Picker UI, `#362` Loading Screen UI & Simlish Voice Catalog, and `#364` REDB Photo Album & Persistent Lot Thumbnail tables), `sims-game` needed end-to-end playable integration across `Game`, `Hud`, `NeighborhoodShell`, `CafSession`, `Mixer`, and the desktop window host while keeping `tests/ratchet.rs` pixel-identical.

## Decisions

### 1. Build Mode Part 2 (`Terrain`, `Water`/`Pool`, `Roof`) & Zero-Impact Ratchet Isolation
- **Subtool & Baseline Wiring:** `Hud::draw_for_lot` switches from `build::draw` to `build::draw_with_catalog` and syncs `BuildState::tool_page`. `Game` parses the house's `HOUS 0` chunk via `RoofState::from_house_view` at load time (`Game::open`, `Game::open_from_store`, and `Game::from_loaded_lot`) and passes that baseline to `plan_roof_*_with_base` and `Lot::roof_state_with_base`, synchronizing `BuildState` on tool entry and Undo/Redo.
- **Interactive Pointer & Preview Pipeline:** `Game` extends `BuildDrag` with `Terrain`, `Water`, and `Pool` variants (with Ctrl-demolish/remove and floating `$cost`/`+$refund` cursor tags) and renders `sims_render::build_preview::{draw_terrain_preview, draw_water_preview, draw_roof_preview_from_catalog}` strictly when Build Mode has the corresponding tool active or the user toggles `UiCamera::Roof` (`roof_view` defaults to `false`). Static Live Mode frames remain bit-for-bit identical for `tests/ratchet.rs`.

### 2. Native Loading Screen & Simlish Voices
- **Loading Screen (`LoadingScreen`):** `NeighborhoodShell` loads all 19 Complete Collection splash bitmaps (`sims_ui::loading::RESOURCES`) and `UIText.iff` rotating captions (`STR# 155` / `STR# 271`), exposes explicit `ScreenKind::Loading` (`begin_loading`, `update_loading`, `tick_loading`, `finish_loading`, `render_loading_frame`), and emits `LoadingAudioCue::{StartLoadLoop, StopLoadLoop}` -> `Input::MusicMode(MusicMode::Load)` across startup, lot entry/leave, community lot edit, and travel transitions while preserving synchronous API compatibility for existing shell callers.
- **Simlish Voices (`VoiceCatalog`):** `Mixer` builds `VoiceCatalog::from_sound_files`, handles `Input::PlayVoice` / `Input::StopVoices` and direct voice paths in `SoundEvent::Play`, and mixes pitch-shifted `VoiceStream` instances scaled by `vox_volume_step()` (with spatial attenuation and pause gating for lot objects, while keeping UI/CAF previews unpaused). `CafSession` emits deterministic `CafVoicePreview` cues on Sim editor entry, appearance changes, Sim save, and explicit preview; `Game::play_sim_voice` forwards live Sim speech through `sound::Feed::update`.

### 3. Photo Album Modal & Persistent Custom Lot Map Thumbnails
- **Photo Album (`PhotoAlbumDialog` + `PhotoAlbumStore`):** `NeighborhoodShell` wires `PhotoAlbumDialog` to both the Camera-panel `CamViewAlbum` (`1105`) button (gated by `album_button_enabled`, enabled in `NeighborhoodShell` lots and disabled in standalone `Game::open` so `tests/ratchet.rs` is untouched) and `F10`, backed by fenced `AlbumFamilyToken` transactions for live framebuffer snapshot capture, blank pages, title/caption editing, reorder, and deletion.
- **Persistent Custom Thumbnails:** `NeighborhoodShell` loads `NEIGHBORHOOD_LOT_THUMBNAILS` on startup, neighborhood switch, map switch, and store refresh; persists updated custom thumbnails via `save_lot_thumbnail_checked` on residential and community-edit saves (skipping live travel visits so community lot templates are never overwritten); and removes them via `delete_lot_thumbnail_checked` on bulldoze.

### 4. Travel Destinations Shell & Shop Picker
- **Sub-Neighborhood Chrome & Community Editing:** `NeighborhoodShell` renders `TravelChrome` over Downtown, Vacation Island, Studio Town, and Magic Town maps, projects active `LotFilter` selections onto `RenderOptions::highlighted_lots`, and opens unoccupied destination lots in Build/Buy edit mode via `open_destination_for_edit` + `Game::from_loaded_lot`.
- **Live Travel & Crash Recovery:** `open_travel_dialog` validates via `validate_travel_plan` and presents `TravelDialog`; confirming runs `depart_to_destination` and wraps the booted destination `Lot` with `active_trip: Some(session)`. Returning (via `return_from_destination`, `Ctrl+H`, or Options → Neighborhood) reconciles state onto the home lot without touching community templates. Entering a lot with a persisted `TRVL` envelope automatically recovers via `inspect_active_trip` / `resume_active_trip`, and `abort_active_trip` restores the frozen home lot cleanly.
- **Shop/Wardrobe Picker (`ShopPickerDialog`):** `Game` hosts `ShopPickerDialog` (`F8` on community/destination lots) and deducts Simoleons or MagiCoins via `mutate_world_via_saved` (which preserves `Lot::loaded`'s `trip_binding_tag` so destination purchases reconcile cleanly on `return_from_destination`).
