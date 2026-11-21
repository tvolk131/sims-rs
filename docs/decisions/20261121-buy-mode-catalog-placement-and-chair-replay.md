# Buy Mode catalog, object placement cursor, and 1,200-update Goth chair replay

## Context

Track 2's Buy Mode recording (`goth-buy-chair-order-20261116`, 1,200 updates on
the Complete Collection 1.91 Goth house) enters Buy Mode at `(93, 458)` between
updates 183 and 184, opens Seating at `(258, 524)`, selects the Werkbunnst All
Purpose Chair (`0x26bfbb29`, `$80`) at `(437, 523)`, and moves the pointer into
the house view at `(480, 380)` before update 1110, creating the chair (object
`260`) and its `"Tile Cursor"` helper (object `261`, GUID `0x00000437`) at fine
lot coordinates `[584, 472, 1]` (tile `(36, 29)`). The capture hits its 1,200-update
limit before the left-click release commits the placement.

Previously, `sims-vm` had `go_live` (`0x416534`) and partial-pass scheduling for
`global[19] != 0`, and `sims-ui` laid out the mode buttons (`Command::SetMode`),
but `sims-game` did not carry out `Command::SetMode`, had no Buy Mode catalog
panel, and `sims-vm` had no `"Tile Cursor"` creation or Buy placement tool.

## Findings from Complete Collection 1.91 `Sims.exe`

- **Mode switching (`0x53a2c3`, `0x4cea5d`, `0x4ce44f`):** UI modes map to
  `global[19]` (`0` Live, `1` Build, `2` Buy, `3` Options, `4` Camera). Entering
  Buy Mode on a residential lot resets the catalog to By-Function top-level
  categories (`0x4cebf4`), sets `global[19] = 2`, and arms tool 7
  (`cTSObjectTool`). Returning to Live Mode runs `go_live` (`0x416534`), marking
  rooms uncounted, refreshing lot statistics, and clearing `global[19]` to `0`.
  During `draw_frame`, the active `PanelMode` is read from `global[19]` rather
  than hardcoded to `PanelMode::Live`.
- **Buy Catalog (`0x4cd149`, `0x4cd9a5`, `0x4cb8e7..0x4cb93b`, `0x5050e4`,
  `0x506765`, `0x507288`, `0x508342`):**
  - Residential Buy Mode includes non-person catalog masters (`sub_index == -1`
    or `master == 0`) with nonzero `OBJD +0x4e` (`room_sort`) and graphics or a
    `TTAB`, reading `CTSS`/`CATS` and falling back to the `OBJD` chunk label
    when empty (`0x457d3b`), sorted by price ascending then raw Windows-1252
    catalog name bytes ascending (`0x4cd9a5` → `0x430100` → `memcmp` at
    `0x6363e0`).
  - Function-sort category masks in UI order `0..8` are
    `[0x01, 0x02, 0x20, 0x08, 0x04, 0x10, 0x80, 0x40]`; function-subsort masks
    are `[0x01, 0x02, 0x04, 0x08, 0x20, 0x40, 0x90, 0xff]`; room-sort masks are
    `[0x08, 0x20, 0x02, 0x80, 0x01, 0x04, 0x10, 0x40]`.
  - The catalog strip sits in the bottom 100 pixels (`y = height - 100..height`);
    the stretched `4918` (`kTallSubpanel`, 150 px) artwork extends 50 px
    transparently above the strip, so hit-testing (`hud::covers`) treats only
    its bottom `STRIP` pixels as covering the lot.
  - Clicking a catalog slot (`0x5065c8`) checks affordability against
    `available_funds` (`0x46d8c9`: `999_999` in budget mode 1, else
    `world.budget.funds`), selects the GUID, and shows the mouse-transparent
    popup card (`cWinCatalogPopup`, `0x508342`) without creating any lot object
    yet.
- **Placement cursor (`cTSObjectTool` vtable `0x667cd0` and `cTSTileCursor`
  vtable `0x666b8c`):**
  - `OnMouseEnter` (`0x487d3d` → `0x486f5a` → `0x421986`) creates the catalog
    object via `0x440b2d` (object `260`, running Init + first Main turn +
    finish, leaving `flags = 9`, `countdown = 19999`, `registry_word = 609823`,
    and `data[45..47] = 1997, 6, 16`), then creates a `"Tile Cursor"`
    (`0x00000437`, type `9`, object `261`) via `0x4215e6` / `0x421afb`.
  - Crucially, `0x4215e6` constructs the `"Tile Cursor"` without stamping the
    lot date (`0x431df3` skipped) and without calling the factory's turn/finish,
    so its `0x431be1` reset computes age from `(0, 0, 0)` (`data[18] = 1997`,
    `data[48] = 1997 * 12 + 6 = 23970`) and leaves `registry_word = 0xffff`
    (`flags = 0`, `countdown = -1`). `0x42231d` copies `data[23]` (turn step)
    and initial facing from the held object, and `0x422776` rotates the held
    object to `[2, 0, 6, 4][camera_rotation & 3]` via `0x421be4` without
    rotating the `"Tile Cursor"` itself (`260.data[1] = 2`, `261.data[1] = 0`).
  - `OnMouseMove` (`0x485a2e`) unprojects the pointer against terrain
    (`0x4a97c3` with flag 1 → `0x4a955d`, aligned to `pick::ORIGIN_COLUMN`),
    snaps to tile centre `(v & !15) | 8` (`0x485f6d`), retains any existing
    surface container when staying on the same snapped tile (`0x421e72` /
    `0x422050` so clicks preserve manual surface rotations), then tests direct
    tile placement and — on collision refusal `0x0b` for a single-tile object
    with nonzero `data[4]` (`+0x52`, allowed surface heights, `0x4220a5`) —
    searches surface container slots (`0x422050`), aligning the placed item's
    facing to the container (or first unblocked cardinal side of a multi-tile
    container via `0x4221e4..0x422280` + `0x43244d` walking the tile chain
    slot `0`). When valid at the snapped tile, it places the held object first
    and `"Tile Cursor"` second (`0x421e72`), which automatically links
    `held.data[26] = cursor`. When snapped placement fails, wall-attached
    (`data[13] & 0x0f != 0`) and pool-edge (`data[42] & 0x800`) objects
    automatically try alternate facings (`0x48736b` + `0x43583e`); if still
    invalid, placement retries the held object (and cursor) at the unsnapped
    pointer coordinates (`0x485f23` → `0x421e72`), leaving only the cursor at
    the unsnapped position when that also fails. Dragging past the 6 px radius
    (`dx^2 + dy^2 > 0x24`) rotates the held object using `0x485d5d..0x485dad`
    (upper-left `6`, upper-right `4`, lower-left `0`, lower-right `2`, minus
    `2 * camera_rotation`; compared against current facing at `0x485da9` before
    subtracting the OBJD `+0x7a` facing offset `0x434f8d`), while movement
    within 6 px continues normal tile placement.
  - `OnMouseLeave` (`0x487d4e` → `0x421963`) takes both off the lot and resets
    cursor position off-lot; `OnMouseUp` (`0x4864d0`) first invokes move at the
    release coordinates (`0x4864f0`), then commits (`0x486ada` → `0x421a0e` →
    `0x43562f` / `0x422e99`, `0x4929fb` → `0x46d91c`), deleting the
    `"Tile Cursor"`, placing the held object permanently, running OBJf function
    11 (`USER_PLACEMENT`) on the object (and every group tile / held child),
    and deducting its price in budget category `6` (Buy) or `7` (Build). When
    Shift is held (`VK_SHIFT`, `0x4864f9`) or for repeat-buy objects
    (`OBJD word 96 == 1` outside tutorial `global[26] == 0`, `0x4878f3`), a
    successful commit immediately arms another copy with the placed facing
    (`0x4864d0` → `0x486f5a`) if affordable. Cancel (`0x487105`) deletes both
    without charging funds; saving mid-preview strips the unpaid preview from
    the snapshot (`Lot::save_without_buy_cursor`).

## Decision

1. **Render (`crates/sims-render/src/cutaway.rs`):** Expose
   `unproject_terrain(lot, camera, at)` using `View::fine` (`0x4a97c3(1)`) and
   `0x47bc1f` unrotation about `1024`.
2. **VM (`crates/sims-vm/src/buy.rs`, `factory.rs`, `boot.rs`):** Add
   `Machine::construct_tile_cursor` (type `9` without date stamp), `BuyCursor`,
   `default_buy_direction`, `snap_to_tile_centre`, `panel_mode`,
   `available_funds`, and `Interpreter`/`Lot` methods (`set_mode`, `buy_start`,
   `buy_move`, `buy_rotate`, `buy_hide`, `buy_commit`, `buy_cancel`). Pass
   `panel_mode(&self.world)` into `draw_frame_in` during `Lot::advance`.
3. **UI (`crates/sims-ui/src/buy.rs`, `command.rs`, `hud.rs`):** Add
   `Command::{BuyCategory, BuySubcategory, BuyBack, BuyPage, BuyItem}`,
   `BuyCatalog`, `BuyState`, `BuyLayout`, `PopupCard`, and Buy Mode strip
   layout/drawing/hit-testing; exclude the transparent top 50 px of `4918` from
   `hud::covers`.
4. **Game (`crates/sims-game/src/hud.rs`, `game.rs`, `main.rs`):** Lazy-load
   `BuyCatalog` on Buy Mode entry, wire `Command::SetMode` and Buy catalog/tool
   hover, drag-rotate, placement commit, cancel (`Escape`), keyboard rotation
   (`,`/`.`), and return to Live mode.

## Verification

- Offline unmasked 1,200-update comparison of `goth-buy-chair-order-20261116`
  in strict mode (comparing tick, scheduler cursor, lot/global RNGs, all 64
  globals, and every object's position, `last_result`, `data`, `temps`,
  `attributes`, frames, person `data`, `motives_bits`, `active`, and `queue`
  across 377 → 345 → 347 objects) matched all 1,200 updates with only three
  known differences:
  - UI `global[12]` (updates `184..=1200`: `0` vs `2008`/`150`/`1500`/`4300`);
  - `obj[259].person.data[37]` marked unknown in `routing.unknown_words`
    (updates `14..=1200`: `0` vs `3287`);
  - `obj[259].frame[2](tree=4099).local[7]` marked unknown in `frame_storage`
    (updates `14..=1200`: `0` vs `2623`).
- Both newly created objects (`260` Werkbunnst Chair and `261` `"Tile Cursor"`)
  matched on every field including position `[584, 472, 1]`, `data`, `frames`,
  `last_result`, and `registry_word` (`609823` and `65535`).
- Added `the_recorded_buy_chair_order_replayed_in_the_window_matches_the_recording`
  and `buy_mode_places_rotates_cancels_and_returns_to_live_mode` in
  `crates/sims-game/tests/goth.rs`, plus unit tests in `crates/sims-ui/src/buy.rs`
  and `crates/sims-ui/src/hud.rs`.

## Scope not yet ported

- Moving or selling already-placed objects in Buy Mode (`cTSObjectTool` hand
  pickup and Delete/sell depreciation, `0x486960`).
- Sort-by-Room / Sort-by-Function toggle buttons in the Buy strip UI (the
  `BuySort::ByRoom` filter and layout exist in `sims-ui::buy`, with
  `BuySort::ByFunction` active by default).
- Buy Mode UI sound effects beyond `UI_error` (placement/pickup sound names not
  yet audited against `Sims.exe`).
- Build Mode tools (walls, floors, terrain, architectural objects).
