# 2026-11-22: UI Honesty Audit Wave 1 — Interests & Gifts Panels, Commercial Buy Catalogs, PopupInfoTiles Hover Chrome, Options Zero-Overlap, LotVisibility, Old Town Travel, and Ordinary VM Dialog Formatting

## Context

The 2026-11-22 UI Honesty Audit uncovered six high-impact P0/P1 UI and catalog fidelity gaps across Live Mode subpanels, Buy/Build catalogs and hover popups, Options controls, travel destination mapping, and ordinary VM dialogs (`Dialog` kinds `0..=4`):

1. **Blank `Panel::Interests` (`4505`) and `Panel::Gifts` (`4507`) strips (`P0-8`):** Clicking Interests or Gifts on human household Sims rendered an empty blue strip (`hud::layout` had no subpanel layout or drawing implementation for either tab).
2. **Dropped commercial expansion Buy Mode objects & truncated rating labels (`P0-9`, `P1-5`):** `BuyCatalog::load` discarded any object whose residential `room_sort` and `function_sort` words were both zero, dropping 121–223 commercial-only expansion objects per zone (cash registers, restaurant podiums, vacation tents, fashion runways, magic stands). `RatingLabels` only read 14 of the 20 entries in `GameData/UIText.iff STR# 160` and masked `OBJD_RATING_SKILLS` (`+0xb2`) with `& 0x7f`.
3. **Flat blue Buy/Build hover popups (`P1-4`):** Buy Mode and Build Mode hover cards painted flat `[20, 32, 92]` rectangles with top+left-only lines instead of the native 9-sliced `PopupInfoTiles.bmp` (`3008`) frame.
4. **Options `"Saves"`/`"Co-op"` overlap, swapped Vacation/Studio comments & tests in `LotVisibility::for_lot`, and `TravelDestination::OldTown` aliasing (`P0-11`, `P0-12`, `P0-13`):**
   - On `800×600` (and narrower viewports), the custom `"Saves"` and `"Co-op"` strip buttons overlapped `"Export HTML"` on the Play tab and the `"Character Detail"` High radio on the Graphics tab.
   - `options.rs` and `tests/options.rs` mislabeled lot range `81..=89` as Vacation Island instead of Studio Town.
   - `ui_destination_for_vm` mapped `TravelDestination::OldTown` onto `Destination::Downtown`.
5. **Unformatted ordinary VM dialogs (`0..=4`), missing icon modes `0, 1, 2, 4`, and empty title when `operands[6] == 0` (`P0-3`):** Ordinary VM dialogs showed raw `$Me`/`$Local`/`$Object`/`$Neighbor`/`$TimeLocal`/`$Job` tokens verbatim, skipped icon modes `0, 1, 2, 4`, and left titles blank instead of falling back to the caller's catalog display name.

## Verification Against `Sims.exe` 1.91 & Complete Collection Assets

1. **Interests & Gifts (`0x50f3c0..0x50f6f0`, `0x51aa96..0x51ac9e`, `0x51e260..0x51e440`):**
   - `UIGraphics.far` supplies 19 `Interest*.bmp` icons (`1600..=1618`, `75×25/26`), `InterestBars.BMP` (`1619`, `40×16`), and `InterestBarsBackground.BMP` (`1620`, `40×16`). Bars draw at `(x + icon_w + 5, y + 5)` with width `points * 4`.
   - `person.data` interest words for indices `0..=18` are `[46, 47, 48, 49, 50, 51, 52, 53, 46, 47, 48, 49, 13, 14, 16, 20, 26, 54, 55]`. Adults display 15 interests (`[0..=7, 12..=18]`); children (`age != 0 && age < 18`) display 8 (`[8..=11, 4..=7]`).
   - Inventory sort buttons in `UIGraphics.far` are 4-cell `80×20` sheets (`4112` `InventoryGiftSort.bmp` for kinds `0, 4, 5`; `4113` `InventoryMagicSort.bmp` for kind `7`; `4114` `InventoryOtherSort.bmp` for kind `8` only, matching `0x51e402`).
2. **Commercial Buy Mode & `STR# 160` (`0x6cc464..0x6cc684`, `0x4cb9cd..0x4cbaa8`, `0x50824b..0x50878e`):**
   - OBJD expansion sort words sit at `+0xbe` (Downtown), `+0xc2` (Vacation), `+0xc6` (Old Town / Community), `+0xce` (Studio Town), and `+0xd0` (Magic Town). All 5 commercial zones use 5 category bits (`[0x01, 0x02, 0x04, 0x08, 0x80]`) and 8 function subcategories.
   - Scanning Complete Collection yields 720 Residential, 703 Downtown, 726 Vacation, 692 Old Town/Community, 601 Studio Town, and 585 Magic Town catalog items.
   - `GameData/UIText.iff STR# 160` holds 20 entries: `[0..=13]` motive/skill ratings plus `[14..=19]` usage/group labels driven by non-debug `TTAB` (`InteractionTable`) flags (`bit 4` no kids, `bit 6` no adults, `bit 9` dogs, `bit 10` cats, and `joining_index >= 0` group activity).
3. **`LotVisibility::for_lot` (`0x5177aa..0x51788e`, `0x40747c`, `0x40749a`, `0x4074b1`, `0x4074c8`, `0x428825`):**
   - Free Will is hidden on Downtown (`21..=30`, `0x40749a`), Studio Town (`81..=89`, `0x4074b1`), and Magic Town (`93..=98`, `0x428825`).
   - Free Will stays visible on Residential lots, Vacation Island (`40..=49`, `0x4074c8` -> `0x51784a`), and Old Town community lots (`zoning == 2`, `0x40747c` -> `0x51784a`).
4. **Ordinary VM dialog formatting & icons (`0x44bf1a`, `0x436219..0x4363b5`, `0x457d3b`):**
   - `0x44bf1a` substitutes `$FamilyAssets`, `$Family`, `$Me`, `$Object`, `$Local:N`, `$$Local:N`, `$Attrib:N`, `$NameAttrib:N`, `$NameLocal:N`, `$NeighborLocal:N`, `$Neighbor`, `$FameTitleLocal:N`, `$Grade`, `$TimeLocal:N`, `$JobOffer`, `$JobDesc`, and `$Job` purely from read-only world/catalog state while recording the portrait target GUID (`[ebp+0x14]`).
   - When `operands[6] == 0`, the title falls back to the caller's localized `CTSS[0]` catalog display name (`0x457d3b`).
   - **Icon modes `0..=4` (`0x436279..0x4363b5`):**
  - Mode `0` resolves the portrait GUID written by `$Object`/`$Neighbor`, else the stack object's master definition GUID (`0x436308..0x43634d`), else `None` (no caller fallback).
  - Mode `1` and Mode `4` leave the icon `None` (`0x436279`; mode `4` formats `operands[1]` text without portrait side effect).
  - Mode `2` resolves `world.neighbors.get(stack_object)` directly (`0x4362f4`, never checking live object IDs first), else `None`.
  - Mode `3` loads `BMP_ (1000 + operands[1])` from the code owner's IFF.
- **Deferred Track 2 (`sims-vm`) follow-ups (out of Track 1 crate scope):**
  - `crates/sims-vm/src/primitives/dialog.rs::format_text` (`1455`) currently traps on `$Family*`, `$Attrib*`, `$NeighborLocal`, `$Grade`, and `$FameTitleLocal` before queuing a dialog; lifting that VM gate to share read-only substitution belongs in a Track 2 (`sims-vm`) lane.
  - Capturing immutable locals/stack-object snapshots at emission time for `informs` dialogs (`operand 7 bit 0`, `crates/sims-vm/src/primitives/dialog.rs:590`) also requires `sims-vm`/`sims-sim` record changes in Track 2.

## Decisions & Changes

1. **`crates/sims-ui/src/command.rs`, `household.rs`, `hud.rs`, `crates/sims-game/src/hud.rs`, `game.rs`:**
   - Added `InventoryTab` (`Gifts`, `Magic`, `Other`), `InventoryEntry`, `INTEREST_WORDS`, `Resident::interests`, `Snapshot::inventory`, and presentation commands `InterestsPage`, `ShowInventory`, `InventoryPage`, and `SelectInventoryItem`.
   - Loaded `1600..=1620` and `4112..=4114` via `hud::OPTIONAL_RESOURCES` and implemented native `Panel::Interests` and `Panel::Gifts` layout, pagination, and thumbnail/badge rendering.
   - Rewrote `compute_options_extra_button_rects` so `"Saves"` and `"Co-op"` dock cleanly beside the divider when no subpanel is open, right-align inside the strip when `>= 8 px` clear of all subpanel controls, and otherwise dock cleanly above the strip (`strip_y - BTN_H - 4`, right of `CLUSTER`, stepping above any active Options balloon/tooltip) with zero overlap at `640×480`, `800×600`, `1024×768`, and `1280×720`.
2. **`crates/sims-ui/src/buy.rs`, `build.rs`, `crates/sims-game/src/hud.rs`, `game.rs`:**
   - Added `BuyZone` (`Residential`, `Downtown`, `Vacation`, `Community`, `StudioTown`, `MagicTown`) and `BuyZone::for_lot`, parsed all 5 OBJD commercial sort words (`+0xbe`, `+0xc2`, `+0xc6`, `+0xce`, `+0xd0`), retained any object with a nonzero residential or commercial sort word, and automatically selected the active zone from the current lot ID and zoning.
   - Expanded `RatingLabels` to all 20 `STR# 160` entries, read full `u16` `OBJD_RATING_SKILLS` (`+0xb2`), and parsed `TTAB` usage restrictions & group activity.
   - Replaced flat `[20, 32, 92]` Buy and Build hover cards with native `dialog::draw_nine_slice` using `PopupInfoTiles.bmp` (`3008`).
3. **`crates/sims-ui/src/options.rs`, `travel.rs`, `crates/sims-game/src/neighborhood.rs`:**
   - Corrected `LotVisibility::for_lot` naming/comments (`is_studio_town` for `81..=89`) and updated `crates/sims-ui/tests/options.rs` to assert Vacation (`40..=48`) shows Free Will while Studio Town (`81..=89`) hides it.
   - Added `Destination::OldTown` (`50..=79`, `PHONE_NEIGHBORHOOD` `5433`, `TravelTransport::Trolley`, Unleashed community filters, `UIText.iff STR# 260`) and mapped `TravelDestination::OldTown => Destination::OldTown` and `OldTownTrolley => TravelTransport::Trolley`.
4. **`crates/sims-ui/src/dialog.rs`, `crates/sims-game/src/hud.rs`:**
   - Added `DialogContext`, `FamilyInfo`, `Message::read_with_context`, `format_text` / `format_dialog_text`, caller display-name title fallback, and exact 1.91 icon-mode dispatch (`0..=4`) without mutating `World` (preserving exact simulation determinism and Goth 43,200-tick hash `df6eb544...`).
5. **Tests:**
   - Created `crates/sims-ui/tests/buy.rs` covering Residential + all 5 expansion commercial catalogs, all 20 `STR# 160` rating labels, and `PopupInfoTiles.bmp` (`3008`) 9-slice hover cards in Buy and Build Mode.
   - Extended `crates/sims-ui/tests/hud.rs`, `dialog.rs`, `options.rs`, `travel.rs`, and `crates/sims-game/tests/options_and_dialogs.rs` (including a full-composite pairwise disjointness test across all 4 Options states and 4 resolutions).

## Verification

- Visually inspected 12 rendered PNGs in `/tmp/wave1-audit-shots/` covering `Panel::Interests` (adult & child), `Panel::Gifts`, Buy & Build `PopupInfoTiles.bmp` 9-slice popups, Downtown & Magic Town commercial Buy Mode strips, Options Play & Graphics tabs at `800×600` and `1024×768`, and a formatted ordinary VM dialog with 45×45 portrait over `SimStub.BMP`.
- Verified all 5 local release gates: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, full `cargo test --release --workspace` (with `SIMS_INSTALL`, `SIMS_UD2_AFTER`, and `SIMS_RATCHET`), `cargo test --release -p sims-game --test ratchet -- --nocapture` (`8/8 ratchet: ok`), and `python3 scripts/check_repository.py --history HEAD` (`0 violations`).
