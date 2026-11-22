# Create-a-Family (`CAF`) Character & Household Persistence Backend (`sims-vm::caf`)

*Decided 2026-11-21.*

Complete Collection 1.91's Create-a-Family (`CAF`) character IFF synthesis,
`NBRS`/`FAMI` household creation, Family Bin deletion, and `.FAM` export/import
now live in `crates/sims-vm/src/caf.rs`, verified byte-for-field against Track
2's Complete Collection 1.91 oracle capture (`caf-create-family-20261121`:
`User00172.iff`, `00-initial/Neighborhood.iff` -> `01-after-create/Neighborhood.iff`,
and `Export/Alice_7.FAM`).

## Architecture & Crate Boundaries

- **`sims-vm` remains strictly decoupled from presentation crates (`sims-people`, `sims-ui`):**
  `sims-people` optionally depends on `sims-vm` (and uses `sims-vm` in
  `dev-dependencies`), so `sims-vm::caf` accepts the six 3D portraits rendered
  by `sims_people::caf_preview::render_portrait` as `sims_formats::bitmap::Image`
  values (`PortraitBitmaps::from_images`) or validated BMP bytes
  (`PortraitBitmaps::from_raw_bmps`), encoding them into uncompressed 24-bit
  bottom-up Windows BMPs (`encode_bmp24`) matching `User00172.iff` header-for-header
  and pixel-for-pixel.
  Headless callers/tests without a renderer may omit portraits to embed valid
  native-dimension placeholders (`PortraitBitmaps::placeholder`).
- **Read-only install, REDB-backed persistence:**
  `CafContext::from_install` reads `TemplatePerson.iff` (`0x7fd96b54`), the
  install's catalog GUIDs, `ImportWardrobe`, and `Global.far` `FCNS #4`
  `"money for new family"` (`$20,000`) strictly read-only. All synthesized
  `User%05d.iff` blobs and updated `StoredNeighborhood` metadata are persisted
  into `NeighborhoodStore` (REDB) without modifying `SIMS_INSTALL` on disk.
- **Three-phase tombstone-shielded commit safety:**
  Because `NeighborhoodStore` currently exposes separate `save_character_file`
  and `save_neighborhood_checked` APIs, `commit_family_with_context` and
  `import_family_fam` serialize in-process CAF writes (`CAF_COMMIT_LOCK`;
  REDB enforces single-process exclusive file locking) and run a three-phase
  protocol: (1) atomically check `(instance_id, revision)` and reserve every
  new GUID in `deleted_characters` before writing any character file (advancing
  across `u64::MAX` revision rollover under checked commits if ever reached),
  (2) write each `User%05d.iff` (invisible to concurrent `load_lot_snapshot` /
  `enter_lot` and safe across mid-write crashes because its GUID is suppressed
  by `deleted_characters`), and (3) atomically publish `FAMI`, `NBRS`, and
  un-tombstoned GUIDs via `save_neighborhood_checked`. Because
  `NeighborhoodStore::save_character_file` is currently an unchecked single-file
  write in `sims-sim`, callers must not race `NeighborhoodStore::import_directory`
  (replacing the same neighborhood ID) against an in-flight CAF commit; a
  follow-up `sims-sim` helper (`save_characters_and_neighborhood_checked`)
  should collapse these phases into a single checked REDB write transaction once
  `crates/sims-sim` is free to modify.

## Complete Collection 1.91 Parity Verified Against `caf-create-family-20261121`

1. **Character IFF (`User%05d.iff`):**
   - Clones all live chunks from `TemplatePerson.iff` (`BHAV` 4096–4100, `TPRP`,
     `TREE`, `SLOT 128`, `GLOB 128`, `STR#` 256/257/258/300/304) except deleted
     chunks and old `BMP_ 2002..=2007`.
   - Patches `OBJD 128` at `0x1c..0x20` (fresh unique GUID) and `0x8c..0x90`
     (`0x7fd96b54`, `TEMPLATE_PERSON_GUID`) with label `user%05d - <first_name>`
     (truncated to 63 Windows-1252 bytes) — byte-for-byte identical to
     `User00172.iff`.
   - Writes localized `-3` `CTSS 2000` (`[first_name, bio]`, 1-byte-per-row
     trailing slack per `0x47090e`, exact 52-byte length for Alice).
   - Writes 35-row localized `-3` `STR# 200` (`bodystring`): skeleton (`adult`/
     `child`), everyday body (`<suit>,BODY=<tex>`), head (`<suit>,HEAD-HEAD=<tex>`),
     bones (`HEAD`/`Top`/`R_HAND`/`Palm`), gender (`male`/`female`), age (`27`/`9`),
     skin tone (`lgt`/`med`/`drk`), Complete Collection formal/swim/sleep/winter
     outfits (`f100`/`s100`/`l100`/`w100` for adults; `ADDED` formal/high-fashion
     + `s100`/`l100`/`w100` for children, including Complete Collection's shipped
     `l100fcchddrk_orignal` sleepwear typo verified in `User00148.iff`), and
     invokes `sims_content::family_import::refresh_outfits` to synthesize nude,
     underwear, six hand poses, formalwear, gloves, and high-fashion rows —
     verified all 35/35 rows verbatim against `User00172.iff`.
   - Embeds all six 24-bit bottom-up BMP portraits (`BMP_ 2002..=2007`,
     `bfSize = 0`, `bfOffBits = 54`, `biSize = 40`, `biBitCount = 24`,
     `biSizeImage = 0`) with native chunk labels (`"faces"`, `"rel. images"`,
     `"newimage"`, `"speech_med"`, `"speech_large"`, `"web_image"`), verified
     header-for-header and pixel-for-pixel against `User00172.iff` (zero-filling
     the 4-byte row-alignment slack where native `Sims.exe` leaves uninitialized
     Win32 heap bytes).

2. **Neighbor Record (`NBRS`):**
   - Version 10 (`version_extra = Some(9)`), `person_mode = 9`
     (`SAVED_REVISION`), `name = "user%05d"`, `unknown_zero = 0`,
     `unknown_negative = -1`, 256 `person_data` words:
     - Asymmetric personality (`0..=10` each, budget `<= 25`) scaled by `100`
       into `[7, 6, 3, 5, 2]` (`Neat`, `Outgoing`, `Active`, `Playful`, `Nice`).
     - 15 initial interests drawn via `crate::objects::Draws::default_traits`
       (`0x459a9c`: words `46..=55` with `[4, 3, 3]` low/mid/high buckets and
       words `[13, 14, 16, 20, 26]` with `[1, 2, 2]` buckets).
     - `person_data[58]` = age (`27` / `9`), `person_data[60]` = skin tone
       index (`1` = `lgt`, `2` = `drk`, `3` = `med` per `PersonGlobals STR# 0x2000`),
       `person_data[61]` = `family_id`, `person_data[65]` = gender (`0` male /
       `1` female), and for children `person_data[56] = -1` (school) and
       `person_data[57] = 4` (starting grade C).
     - `person_data[70]` (zodiac) remains `0` in `NBRS` (matching
       `01-after-create/Neighborhood.iff`), while `compute_zodiac` reproduces
       `0x4841fc`'s `f32` nearest-sign calculation (`Alice (7,5,3,4,6)` -> `4`
       Cancer).

3. **Household (`FAMI`) & Family Bin Commit / Delete:**
   - Allocates lowest unused positive non-reserved `family_id` (`7` on
     `00-initial`), `number = max(existing.number, 0) + 1` (`16`), `version = 9`,
     `house = 0`, `budget = $20,000`, `architecture_value = 0`, `friends = 0`,
     `flags = 0x18` (`FAMILY_FLAG_BIT_3 | FAMILY_FLAG_CAF_CREATED`), 1..=8
     members with at least one adult human Sim, and 16-byte zeroed v9 tail —
     verified byte-for-byte against `FAMI 7` (60 bytes) and length-/field-level
     against `FAMs 7` (13 bytes, zero-filling the 1-byte native trailing slack).
   - Fresh GUID allocation (`0x4725ff` high/low word structure) avoids `0`,
     `TEMPLATE_PERSON_GUID`, catalog GUIDs, existing character file OBJDs
     (both loose `.iff` and `.far` archives), `NBRS`, `FAMI`, `made_characters`,
     `deleted_characters`, and `deleted_at_epoch`.
   - `delete_bin_family` rejects reserved pool families (`0`, `2000`,
     `3000..=3009`, `4000`, `5000`, `5500`, `6000`) and housed families, removes
     the `FAMI` entry, vacates unshared members' `NBRS` slots, scrubs reciprocal
     relationships and inventories, bumps `deletion_epoch`, and tombstones
     deleted GUIDs in `deleted_characters` + `deleted_at_epoch` so subsequent
     lot loads never resurrect deleted Sims.

4. **`.FAM` Export & Import Helpers:**
   - `export_family_fam` writes Complete Collection `.FAM` archives (`FAMI`,
     `FAMs`, `CTSS #1000`, `EXPi #0`, per-member `uChr`/`CTSS`/`CTSS+2000`/`BMP_`,
     `NBRS #1`, and optional `FINV #2`), rejecting colliding `CTSS` chunk IDs
     before serialization and verified chunk-for-chunk against `Export/Alice_7.FAM`.
   - `import_family_fam` imports Family Bin `.FAM` archives (including Track 2's
     `Export/Alice_7.FAM`) into `NeighborhoodStore` following native
     `family_import` ID/slot retention, revision-gated `NBRS` field transfer,
     relationship deduplication/remapping, `architecture_value` preservation,
     24-bit BMP normalization, and `FINV #2` inventory decoding & remapping.

## Consequences

- Newly created CAF households immediately appear in `StoredNeighborhood::family_bin()`,
  pass `sims_vm::lifecycle::move_in`, spawn onto vacant residential lots (`House04`),
  step 500+ ticks cleanly under `Frames::default()`, and round-trip through
  `save_and_leave_lot` and re-entry.
- `StoredNeighborhood` does not yet store a dedicated `FAMs` name/bio map;
  `commit_family` returns `CreatedHousehold { family_name, family_bio, fams_chunk, .. }`
  so callers can display or persist family names alongside `NeighborhoodStore`
  until a follow-up `sims-sim` schema revision adds `FAMs` storage.
