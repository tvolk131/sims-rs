# Complete Collection UI Polish & Fast Save Manager

Date: 2026-11-22
Branch: `game/polish-tryon-books-nb-buttons-and-fast-saves`
Scope: `crates/sims-ui/`, `crates/sims-game/`, and this ADR (strictly disjoint from core simulation, VM, render, people, format, and content library crates).

## Context

Following Performance Part 4 (`#378`) and the Special Dialog Panels & Animations lane (`#376`), five high-value player-facing gaps and two Track 1 P2/P3 UX polish items remained inside the UI and game shell layers:

1. **Save Manager listing overhead**: `build_save_manager_snapshot` still ran `AutosaveStore::list_saves_validated(..., validate_save_payload_cached)`, decompressing V2 payloads and deserializing every slot's `Saved` / `World` synchronously on the UI thread before displaying slot cards, even though `sims-sim` now exposes zero-decompress `AutosaveStore::list_saves_metadata`.
2. **Wardrobe & Charm Try-On preview**: `ShopCommand::TryOn` inside `ShopPickerDialog` (`WardrobeShop` kind 6, `CharmTransform` kind 14, and interactive boutiques) only marked `tried_on_item_id` and set a status note rather than rendering a live 3D Sim preview.
3. **Truncated Spellbook & Cookbook notice**: `SpecialPanelRequest::SpellBook` (kind 12) and `CookBook` (kind 15) used a single 6-line notice dialog showing only caller inventory items instead of Complete Collection's paginated reference catalog from `GameData/magic.iff`.
4. **Unported Neighborhood top-bar secondary actions**: `ToggleRezone`, `ImportFamily`, `OpenCredits`, and `OpenWeb` emitted status notes instead of interactive native flows.
5. **Track 1 P2/P3 UX polish**:
   - Edit-mode `SaveCardQuit` failure upgraded `ConfirmQuit` directly into `ConfirmSaveBeforeQuit` without first explaining the write failure in a native notice dialog.
   - Open text-entry carets (`CheatConsole`, `PhotoAlbumDialog`, `DialogState`, `CoopLobbyDialog`) still polled at 100 ms (`CARET_POLL_US`) or 500 ms unconditionally rather than scheduling exact caret blink transitions (and `None` when unfocused).

## Decision

### 1. Fast Save Manager Listing (`crates/sims-game/src/autosave_worker.rs`, `crates/sims-game/src/neighborhood.rs`)
- Switched `build_save_manager_snapshot` and the corrupt-lot-open fallback check in `NeighborhoodShell::activate_lot` to `AutosaveStore::list_saves_metadata`, verifying every slot's outer SHA-256 envelope, format version, and key-bound metadata header without decompressing V2 payloads or deserializing full `World`/`Saved` bodies on the UI thread.
- Kept full `Saved::decode` / `World::decode` validation (`validate_save_payload`) on actual slot loads (`load_save_validated`), descending tick/timestamp candidate recovery (`FallbackToNewestIntact`), and background worker commits (`commit_job_validated(..., validate_save_payload_cached)`).

### 2. Live 3D Sim Preview for Wardrobe & Charm Try-On (`crates/sims-ui/src/shop.rs`, `crates/sims-game/src/game.rs`)
- Extended `ShopSnapshot` with `preview_image: Option<Image>`, `preview_label: Option<String>`, and `buy_label_override: Option<String>`, and added `ShopCommand::RotatePreview(i16)` plus `<` / `>` rotate buttons inside a responsive right-hand preview pane in `ShopPickerDialog` whenever Try-On is active.
- Wired `Game` to bind each Try-On item explicitly to its wardrobe/charm/category outfit (`ShopPreviewTarget`), render live 3D Sim previews via the same `sims_people::caf_preview` pipeline (`SimConfig::load_body` + `render_turntable`) used by Create-a-Sim `CafSession`, cache the loaded `Body` across overflow-safe ±45° rotation clicks/arrow keys, and show `"Preview unavailable"` cleanly whenever an item has no resolvable outfit asset.

### 3. Paginated Spellbook & Cookbook Reference Modal (`crates/sims-ui/src/book.rs`, `crates/sims-game/src/game.rs`)
- Added native 9-slice `sims_ui::book::ReferenceBookDialog` (`3008`, `17`, `5107`) with Previous / Next buttons, localized page counter, within-page vertical scrolling (`^`/`v` buttons, `Up`/`Down`, mouse wheel) so wrapped text is never clipped at short window heights, host-owned `BookCommand::Close` dismissal so VM special panels always receive `SpecialPanelResponse::Dismissed`, and a held-ingredient/token badge strip.
- Loaded all titles, descriptions, and navigation labels dynamically at runtime from `GameData/magic.iff` (`STR# 1`, `8`, `9`, `10`, `12`, `13`, `14`) and `GameData/UIText.iff` (`STR# 142`) with zero hardcoded game prose.
- Cross-matched each spell/charm/recipe against the calling Sim's `world.neighbors.inventories` (kinds `7..=9`, resolved via catalog `OBJD`/`CTSS`) to highlight held ingredients/tokens both in the top summary strip and on matching pages.

### 4. Neighborhood Top-Bar Secondary Actions (`crates/sims-game/src/neighborhood.rs`, `crates/sims-ui/src/book.rs`)
- **`ToggleRezone`**: Toggles `NeighborhoodMode::Rezone` (enabling hovered-lot highlighting), validates eligible main-neighborhood lots (`1..=10`, `50..=80`), rejects rezoning either endpoint of an active travel trip, binds confirmation to `(instance_id, revision, occupant)`, stages any occupied-lot eviction and zoning update together in an ephemeral store so both commit atomically in a single fenced `save_lot_and_neighborhood_checked` transaction (or `save_neighborhood_checked` for vacant lots), and refreshes map overlays/hover cards immediately.
- **`ImportFamily`**: Scans `UserData*/Import/` and `Export/` (plus optional extra search directories, refreshing availability on window focus), updates `NeighborhoodState::has_importable_family`, opens a native picker modal, normalizes house-bound `.FAM` archives to Family Bin form (`house = 0`) by rewriting only effective `resource_order` chunks (preserving native `rsmp` lookup semantics), imports cleanly into REDB via `sims_vm::caf::import_family_fam`, persists the surname sidecar, and refreshes the Family Bin immediately.
- **`OpenCredits`**: Reuses `ReferenceBookDialog` to display paginated credits loaded dynamically from `GameData/Credits.iff` (`STR# 171..=163`, fallback `UIText.iff STR# 163`) and enables the native bottom-left Credits billboard button on main neighborhood maps.
- **`OpenWeb`**: Opens a clean native `ShellDialogKind::Notice` modal explaining that this offline engine does not launch external web browsers.

### 5. Track 1 P2/P3 UX Polish (`crates/sims-ui/src/{cheat_console.rs, album.rs, dialog.rs, coop.rs}`, `crates/sims-game/src/{game.rs, hud.rs, neighborhood.rs, main.rs}`)
- **Chained Save & Quit failure notice**: Added `Game::after_save_failed_with_notice` + `pending_after_notice` so a failed save after edit-mode `SaveCardQuit` first displays the exact write-failure `OptionsDialog::Notice`, and upon dismissal chains directly into `OptionsDialog::ConfirmSaveBeforeQuit` (while preserving `Game::after_save_failed` compatibility).
- **Exact caret `next_blink_us`**: Exposed `next_blink_us` on `CheatConsole`, `PhotoAlbumDialog` (plus `sync_blink`), `DialogState`, and `CoopLobbyDialog`, removing the 100 ms `CARET_POLL_US` fixed polls from `Hud::next_blink_us`, `Game::next_wake_us_with_window`, and `NeighborhoodShell::next_wake_us_with_window`. An open unfocused Photo Album now schedules `None` (0 wakeups/sec), and focused text carets wake at exact 500 ms transitions.

## Verification & Measurements

- **Measured Save Manager listing time across 6+ slots (`release`, Complete Collection Lot 5 Goth, 6 live slots — 1 manual + 5 rotating autosaves):**
  - Before (`list_saves_validated` + full `Saved::decode`): **11.93–12.01 ms**
  - Before (`list_saves_validated` + cached SHA-256 decompress): **13.69–13.88 ms**
  - After (`list_saves_metadata` zero-decompress envelope + header verification): **0.84 ms** (**~14.2× faster** than full decode, **~16.3× faster** than cached decompress; corruption detection for truncated/tampered envelopes preserved and full `Saved::decode` still runs on load/fallback/commit).
- **All 5 local release gates verified:**
  1. `cargo fmt --all -- --check`
  2. `cargo clippy --workspace --all-targets -- -D warnings`
  3. `cargo test --release --workspace` (including `sims-ui` `ui_ratchet` 4/4, `book`, `shop` 7/7, and `sims-game` `polish_tryon_books_nb_and_fast_saves` 5/5)
  4. `cargo test --release -p sims-game --test ratchet -- --nocapture` (`8/8 ratchet: ok`, zero lowered floors)
  5. `python3 scripts/check_repository.py --history HEAD` (0 violations)
