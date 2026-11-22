# Atomic Single-Transaction CAF Commit, Photo Album & Lot Thumbnail REDB Storage, and `Saved` Forward Migrations

*Decided 2026-11-22.*

Closes the remaining persistence and forward-migration backend gaps in
`crates/sims-sim` and `crates/sims-vm` without touching live gameplay, rendering,
or UI crates.

## 1. Atomic Single-Transaction CAF Commit

PR #355 (`sims-vm::caf`) previously committed synthesized `User%05d.iff` files
and updated `StoredNeighborhood` metadata in a three-phase staged protocol
because `NeighborhoodStore::save_character_file` was an unchecked single-file
write. That left a narrow Phase 2 window against a concurrent
`NeighborhoodStore::import_directory` replacing the same neighborhood ID.

- **`NeighborhoodStore::save_characters_and_neighborhood_checked`**
  (`crates/sims-sim/src/neighborhood_store.rs`) validates non-empty, duplicate-free
  character filenames up front and then inside a **single REDB write transaction**:
  1. Loads and decodes `NEIGHBORHOODS[id]` (failing if missing or corrupt).
  2. Verifies `(current.instance_id, current.revision) == (expected_instance_id, expected_revision)`,
     aborting with `Ok(Err(CheckedCommitError::InstanceMismatch | RevisionMismatch))`
     and **zero** writes on conflict.
  3. Advances `(instance_id, revision)` via `advance_version` (handling `u64::MAX`
     revision rollover atomically inside the same transaction).
  4. Inserts all `(id, filename) -> bytes` entries into `NEIGHBORHOOD_CHARACTERS`
     and updates `NEIGHBORHOODS[id]` together.
- **`sims-vm::caf`** (`crates/sims-vm/src/caf.rs`) replaces the three-phase
  staging helper with `persist_household_atomic`, calling
  `save_characters_and_neighborhood_checked` for both `commit_family_with_context`
  and `import_family_fam`. Each household commit now advances revision by +1
  atomically with zero staging tombstones and zero partial-write window against
  concurrent `import_directory`, `delete_neighborhood`, lot saves, or crashes.
- **`delete_bin_family`** also cleans up the deleted family's persisted photo
  album (fenced by the committed `instance_id`) so a later CAF household reusing
  that `family_id` cannot inherit stale photos.

## 2. Household Photo Album REDB Storage (`sims-sim::album_store`)

`PhotoAlbumStore` (`Clone + Send + Sync`, obtained via `NeighborhoodStore::albums()`
sharing `Arc<Database>`) persists per-`(neighborhood_id, family_id)` household
photo album / storybook entries matching `sims-ui::album`'s commands (`add`,
`update_text`, `move_entry`, `delete`, `list`, plus `get`, `list_families`, and
`delete_family_album`).

- **Tables:**
  - `photo_album_index` (`(&str, u16) -> &[u8]`): framed `"ALBI"` v1 record
    holding `(neighborhood_id, family_id, next_photo_id, order: Vec<u64>)`.
    Keeping ordering in the small index row lets `move_entry` reorder pages
    atomically without rewriting photo bitmaps, while preserving strictly
    monotonic non-reused `photo_id >= 1` across deletions.
  - `photo_album_photos` (`(&str, u16, u64) -> &[u8]`): framed `"ALBP"` v1
    record holding `(neighborhood_id, family_id, id, title, caption, sim_day, image: Option<AlbumImage>)`.
- **Integrity & Bounds:** Both record types carry a 4-byte magic, 4-byte
  little-endian format version (`ALBUM_FORMAT = 1`), 32-byte domain-separated
  SHA-256 checksum, strict exact-length `postcard` decoding
  (`decode_postcard_exact`), and full key-binding verification. Text and bitmap
  bounds are enforced on both write and read: `MAX_ALBUM_TITLE_CHARS = 80`,
  `MAX_ALBUM_CAPTION_CHARS = 512`, `MAX_ALBUM_PHOTO_DIM = 1024`,
  `MAX_ALBUM_PHOTO_PIXELS = 1024 * 768` (stored as row-major 16-bit RGB565
  matching the Sims 1 display color space with `from_rgba` / `to_rgba` helpers),
  and `MAX_ALBUM_ENTRIES = 256` per household.
- **Lifetime Fencing & Purge:** Every mutating method requires a non-zero
  `expected_instance_id` verified against `NEIGHBORHOODS[neighborhood_id]` inside
  the REDB write transaction (`AlbumError::StaleNeighborhoodInstance` on delete
  or re-import), without bumping `StoredNeighborhood::revision`. Both
  `NeighborhoodStore::delete_neighborhood` and `import_directory` purge all album
  rows for that neighborhood inside their atomic write transaction.

## 3. Persistent Custom Lot Map Thumbnails (`NEIGHBORHOOD_LOT_THUMBNAILS`)

Adds a dedicated REDB table `neighborhood_lot_thumbnails` (`(&str, u32) -> &[u8]`)
and checked API on `NeighborhoodStore`:

- `save_lot_thumbnail_checked(id, expected_instance_id, lot_id, thumbnail)`
- `load_lot_thumbnail(id, lot_id)` and `load_lot_thumbnails(id)`
- `delete_lot_thumbnail_checked(id, expected_instance_id, lot_id)`

Each record stores `StoredLotThumbnail { normal: ThumbnailSurface565, roofs_off: Option<ThumbnailSurface565> }`
(matching `sims-render`'s `Surface565` and `MAX_LOT_THUMBNAIL_DIM = 1024`) inside
a framed `"THMB"` v1 envelope with a 32-byte domain-separated SHA-256 checksum,
`decode_postcard_exact`, and `(neighborhood_id, lot_id)` key-binding verification.
Writes and deletes fence on `expected_instance_id` inside the REDB write
transaction without bumping `StoredNeighborhood::revision` (so cosmetic map
thumbnail updates never invalidate active lot sessions), and both
`delete_neighborhood` and `import_directory` purge all thumbnails for that
neighborhood atomically.

## 4. Explicit `Saved` Forward Schema Migrations (`FORMAT 6 -> 7 -> 8`)

Reuses `sims_sim::MigrationRegistry` and `sims_sim::decode_postcard_exact` in
`crates/sims-vm/src/boot/save.rs` (`MIN_FORMAT = 6`, `FORMAT = 8`,
`SAVED_MIGRATIONS`):

- **`v6 -> v7`:** Strictly decodes `SavedV6` (including historical `AnswersV6`
  without `choices_after`), reconstructs `Answers` preserving `queue` and
  `include_single_button` with `choices_after = None`, and initializes
  `deletion_epoch = 0`.
- **`v7 -> v8`:** Strictly decodes `SavedV7` (preserving `Answers::choices_after`
  and `deletion_epoch`) and initializes the 14 format-8 auxiliary `World`
  portrait/outfit/index/scratch tables to empty defaults.
- **Strict validation & compatibility:** `Saved::encode` (`FORMAT = 8`) is
  bit-for-bit unchanged. `Saved::decode` and `Saved::decode_with_report`
  migrate `6..=8` deterministically while verifying `SNAPSHOT_FORMAT == 32` and
  rejecting truncated payloads, trailing bytes, older formats (`< 6`), and
  unsupported future formats (`> 8`). `Saved::decode_strict` accepts only
  current `FORMAT = 8` without migration.

## What Would Reopen This

- If `Saved` advances to `FORMAT = 9`, append a `v8 -> v9` `MigrationStep` to
  `SAVED_STEPS` and bump `FORMAT` without changing `MIN_FORMAT = 6`.
- If `sims-sim` later introduces `ALBUM_FORMAT = 2` or `LOT_THUMBNAIL_FORMAT = 2`
  (e.g., lossless compression or extra metadata), route their decoders through
  `MigrationRegistry` while keeping the outer SHA-256 envelope intact.
