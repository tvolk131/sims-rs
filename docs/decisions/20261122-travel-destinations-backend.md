# Away-From-Home Travel Destinations Backend (`sims_vm::travel`)

*Decided 2026-11-22.*

## Context

Build-order step 3 of [docs/vision.md](../vision.md) adds away-from-home travel
across all Complete Collection 1.91 destinations (**Downtown**, **Vacation
Island**, **Old Town**, **Studio Town**, and **Magic Town**) on top of
`sims_sim::neighborhood_store::NeighborhoodStore` (REDB persistence), without
mutating `SIMS_INSTALL` on disk and without touching sibling tracks' crates or
modules.

## Design

### 1. Destination Classification & Complete Collection Eligibility

`TravelDestination::classify(house, zoning)` maps Complete Collection lot
numbers and `LotZoning.iff` (`1` residential, `2` community) to five
destination zones:

| Destination | Lot IDs | Expansion Bit (`[0x6e48bc]+0x66`) | Default Transport & Fare | Party Rule |
|---|---|---|---|---|
| `Downtown` | `21..=30` | `0x04` (Hot Date) | `DowntownCab` (`$50`) | Single adult human or adult human + adult human date companion |
| `VacationIsland` | `40..=48` | `0x08` (Vacation) | `VacationWagon` (`$500`) | Whole household on home lot (adult leader; children & pets travel too); supports `HotelCheckIn { grant_hotel_key }` and `Campground` |
| `OldTown` | `50..=79` where `zoning == 2` | `0x20` (Unleashed) | `OldTownTrolley` (`$0`) | Whole household on home lot (human leader; pets travel too) |
| `StudioTown` | `81..=89` | `0x40` (Superstar) | `StudioShuttle` (`$50`) or `StudioLimo` (`$0`, fame >= `10`) | Single adult human household member |
| `MagicTown` | `90..=98` (`90..=92` residential, `93..=98` community) | `0x80` (Makin' Magic) | `MagicHole` (`$0`) or `MagicBlimp` (`$50`) | Single human Sim (adult or child) or household group/family |

Validation runs in `validate_travel_plan` before any mutation:
- Rejects self-travel (`destination == home`), non-destination residential lots
  (e.g. `1..=10` or residential `50..=79`), missing expansion bits, occupied
  destination lots (`StoredNeighborhood::family_in(dest).is_some()`),
  departing from a house the family does not reside on (`family.house !=
  home_house`), departing while already away (`family.secondary_house() != 0`),
  mismatched transport/accommodation, invalid party composition (duplicate
  GUIDs, deleted characters, non-member leader, age/species violations,
  insufficient fame for `StudioLimo`), negative fares, and insufficient
  household funds.
- `family_magic_coins` sums `Magic Token - Currency` (`0x99e81bec`, inventory
  kind `7`, verified from `ExpansionPack7.far::TokensMagic.iff` and
  `CartDragon.iff`/`CartFaerie.iff` `Manage Inventory` opcode 51 calls) across
  all household members' inventories (`cFamily::GetMagicCoins`, `0x410282`).

### 2. Departure, Home-Lot Freeze, and Destination Boot (`depart_to_destination`)

To preserve crash/error atomicity, `depart_to_destination` stages the frozen
home lot and boots the destination lot in memory **before** committing to REDB:

1. **Frozen Home Snapshot:** Captures live traveler state (`motives`,
   `previous_motives`, 256-word `person.data`, `person_outfits`, `made_outfits`,
   pre-trip hotel-key presence), snapshots the home lot at its exact departure
   tick and clock (`hours/minutes/seconds`), deducts the fare (recording into
   today's budget entry), flushes all live home persons via
   `people::save_to_record` (`0x425a69`), updates lot values (`0x416085`),
   writes back `StoredNeighborhood`, and sets `family.secondary_house =
   destination_house` (`0x425d95`, FAMI `+0x13c`).
2. **Destination Boot:** Loads `destination_house` from REDB's `saved_lot`
   (preserving prior community-lot edits, running `Load` callbacks on newly
   reconciled global controllers so zone-gated controllers like Superstar's
   `Controller - Insanity` `0xC2AF7E1C` self-remove cleanly on non-Studio lots)
   or its imported base `HouseXX.iff` (validating `OBJT` catalog coverage and
   stripping deleted character instances). For base-house loads with
   `globals[23] < 64` (all destination lots), populates bare-lot architecture
   arrays before `reconcile_controllers` (`0x415e20..0x415e67`), matching
   `boot.rs` so Vacation/Downtown/Studio/Magic controllers initialize cleanly.
3. **Live Away-From-Home Mode:** Shipped destination `HouseXX.iff` files save
   `globals[32] = 1` (Edit/Build unlimited-budget mode). `depart_to_destination`
   switches `globals[32] = 0`, unpauses (`globals[17] = 0`), sets live mode
   (`globals[19] = 0`), selects the visiting household (`globals[9] =
   home_family`, `globals[10] = destination_house`, `budget.funds =
   funds_at_arrival`), and ensures `budget.days[0]` is initialized so native
   `Alter Budget` purchases work on every destination lot.
4. **Party Injection & Check-In:** Issues `Token - Hotel Key` (`0x3ac022d6`,
   `kind = 0`) on Vacation `HotelCheckIn { grant_hotel_key: true }` so native
   category 4/7 hotel doors and beds unlock (`0x458319`), spawns each traveler
   via `Interpreter::create_person` on distinct walkable tiles near the arrival
   portal (`PedPortal` `0x81e6bef9`, `CarPortal` `0x865a6812`, `Magic - Clown
   Portal` `0x2baa0029`, or `Ped Marker Magic - Cab Stop` `0x13076784`), overlays
   their exact live motives/skills/career/fame/outfits (temporarily zeroing
   `data[81]`/`data[77]` across `put` to avoid unported celebrity star-marker
   traps at `0x447a82`), and selects the party leader.
5. **Checked REDB Commit:** Commits `(frozen_home_bytes, staged_stored)` via
   `save_lot_and_neighborhood_checked` only after destination boot succeeds,
   binding `destination_lot.neighborhood_origin` to the committed version.

### 3. Return Home & State Reconciliation (`return_from_destination`)

1. **Session Verification:** Verifies `destination_lot` belongs to the expected
   store, neighborhood, revision, destination house, and visiting family, and
   that REDB's `StoredNeighborhood` still records `family.house == home_house`
   and `family.secondary_house() == destination_house`.
2. **Flush & Extract:** Flushes every live Sim on `destination_lot` into `NBRS`
   (preserving updated skills, career/fame, and relationships with newly met
   townies, vacationers, celebrities, and magic NPCs), revokes temporary
   Vacation check-in hotel keys (preserving pre-existing keys and all
   souvenirs/gifts/ingredients/MagiCoins), marks the returning family's friend
   count stale so `FAMI.friends` is freshly recomputed, and captures returning
   live motives, person data, outfits (`person_outfits` and `made_outfits`),
   and final funds/MagiCoins.
3. **Community-Lot Edit Preservation & Single-Transaction Return:** Matching
   `Sims.exe` 1.91, leaving an away-from-home Live-Mode visit does **not**
   overwrite REDB's destination lot snapshot with transient visit mess,
   spawned NPCs, or mid-visit controllers; community-lot Build/Buy edits are
   authored via `open_destination_for_edit` + `lifecycle::save_and_leave_lot` and stay
   cleanly preserved across all visits. `return_from_destination` writes back
   `FAMI` (updating `budget` and `friends`, preserving home `house` and
   `architecture_value`, clearing `secondary_house = 0`), `NBRS`, `NGBH`
   (keeping `data[2]` pointed at `home_house`), `TATT`, and `made_characters`.
4. **Home Reconciliation & Atomic Commit:** Restores the frozen home lot at its
   exact departure tick and clock (`world.ticks`, `Lot::ticks`,
   `globals[0/5/6]`), reloads updated neighborhood records/relationships/
   inventories/MagiCoins/funds, keeps non-traveling Sims/pets untouched,
   deduplicates and resets returning household travelers cleanly onto distinct
   walkable tiles near the home curb/portal with their updated live motives/
   skills/fame/outfits, and selects the returning leader. Commits the
   reconciled home lot and updated `StoredNeighborhood` in a **single atomic
   checked REDB transaction** (`save_lot_and_neighborhood_checked`), then
   invalidates `destination_lot.neighborhood_origin` to prevent stale
   overwrites.

## Known VM Follow-Ups (Outside Disjoint Scope)

While ticking live Complete Collection destination lots, two unported VM
primitives outside this PR's disjoint `travel.rs` scope were observed:
- `Find Best Interaction` on community/Vacation (`zoning == 2 || 40..50`) and
  unzoned (`zoning == 0`, Downtown/Studio Town) lots (`autonomy.rs:434-445`).
- `Generic Sims Call` routine `26` on Magic Town community lots
  (`primitives/generic.rs:104`).

Home lots (`House05.iff`) and Magic Town residential lots (`House90.iff`,
`zoning == 1`) tick all 220+ ticks in strict `Lot::step()` with zero errors.
Community/unzoned destination lots (`House21`, `House40`, `House58`, `House81`,
`House93`) tick all 220+ ticks cleanly under `Lot::step_play()` with every
caught object fault verified to be strictly `Fault::Trap(Trap::Unsupported(_))`
(zero `Trap::Invalid`, zero `Trap::Native`, zero corrupted slots/objects).

## Verification

- Pure unit tests and direct-mutation Complete Collection round-trip tests in
  `crates/sims-vm/src/travel.rs` verify destination classification, expansion
  gating, transport/accommodation validation, party age/species/fame rules,
  MagiCoin counting, Downtown Date with a non-household townie companion,
  Studio Town celebrity limo qualification, and full reconciliation of motives,
  skills, fame, outfits/`made_outfits`, funds, MagiCoins, gift tokens, and
  bidirectional relationships.
- Live-install Complete Collection integration tests in
  `crates/sims-vm/tests/travel.rs` verify end-to-end travel round-trips from
  Goth `House05.iff` to Downtown (`House21.iff`, including community-lot edit
  persistence across two visits), Vacation Island (`House40.iff` whole-family
  hotel check-in/out), Studio Town (`House81.iff`), Magic Town (`House90.iff`
  strict 220 ticks + `House93.iff` child blimp), and Old Town (`House58.iff` +
  atomicity/session error guards), ticking home cleanly in strict mode after
  every return.
