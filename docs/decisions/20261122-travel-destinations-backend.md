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
- Uses the install's real detected expansion bitmask (`Install::detect` intersected
  with `home_lot.program.expansions`) rather than a hardcoded mask.
- Rejects self-travel (`destination == home`), non-destination residential lots
  (e.g. `1..=10` or residential `50..=79`), missing expansion bits, occupied
  destination lots (`StoredNeighborhood::family_in(dest).is_some()`),
  departing from an unlimited-budget edit lot (`globals[32] != 0`),
  departing from a house the family does not reside on (`family.house !=
  home_house`), departing while already away (`family.secondary_house() != 0`),
  mismatched transport/accommodation, invalid party composition (duplicate
  GUIDs including duplicate leader GUIDs, deleted characters, non-member leader,
  age/species violations, insufficient fame for `StudioLimo`), negative fares,
  and insufficient household funds.
- `family_magic_coins` sums `Magic Token - Currency` (`0x99e81bec`, inventory
  kind `7`, verified from `ExpansionPack7.far::TokensMagic.iff` and
  `CartDragon.iff`/`CartFaerie.iff` `Manage Inventory` opcode 51 calls) across
  all household members' inventories (`cFamily::GetMagicCoins`, `0x410282`).

### 2. Departure, Home-Lot Freeze, `TRVL` Envelope, and Destination Boot (`depart_to_destination`)

To preserve crash/error atomicity and protect both the frozen home lot and the
clean Build/Buy destination snapshot, `depart_to_destination` stages the frozen
home lot and boots the destination lot in memory **before** committing to REDB:

1. **Frozen Home Snapshot:** Captures live traveler state (`motives`,
   `previous_motives`, 256-word `person.data`, `accessories`, `person_outfits`,
   `made_outfits`, pre-trip hotel-key presence), snapshots the home lot at its
   exact departure tick and clock (`hours/minutes/seconds`), ensures
   `budget.days[0]` exists before deducting the fare (recording the actual
   deducted fare into `today[5]`), flushes all live home persons via
   `people::save_to_record` (`0x425a69`), updates lot values (`0x416085`),
   writes back `StoredNeighborhood`, and sets `family.secondary_house =
   destination_house` (`0x425d95`, FAMI `+0x13c`).
2. **Destination Boot:** Loads `destination_house` from REDB's `saved_lot`
   (stripping orphaned/deleted saved persons and preserving every saved
   furnishing and controller from the initial base load, matching
   `lifecycle::boot_from_saved_lot`) or its imported base `HouseXX.iff`
   (validating `OBJT` catalog coverage, promoting `slot 0` successors and
   resetting affected surviving threads when stripping deleted character
   instances, and populating bare-lot architecture arrays before
   `reconcile_controllers` on shrunken lots per `0x415e20..0x415e67`).
3. **Live Away-From-Home Mode:** Shipped destination `HouseXX.iff` files save
   `globals[32] = 1` (Edit/Build unlimited-budget mode). `depart_to_destination`
   switches `globals[32] = 0`, unpauses (`globals[17] = 0`), sets live mode
   (`globals[19] = 0`), selects the visiting household (`globals[9] =
   home_family`, `globals[10] = destination_house`, `budget.funds =
   funds_at_arrival`), and ensures `budget.days[0]` is initialized so native
   `Alter Budget` purchases work on every destination lot.
4. **Party Injection, Date Link & Check-In:** Issues `Token - Hotel Key`
   (`0x3ac022d6`, `kind = 0`) on Vacation `HotelCheckIn { grant_hotel_key: true }`
   so native category 4/7 hotel doors and beds unlock (`0x458319`), spawns (or
   deduplicates and resets) each traveler on distinct walkable tiles validated
   by native `Interpreter::search` / `Interpreter::fits` near the arrival portal
   (`PedPortal` `0x81e6bef9`, `CarPortal` `0x865a6812`, `Magic - Clown Portal`
   `0x2baa0029`, or `Ped Marker Magic - Cab Stop` `0x13076784`), overlays their
   exact live motives/skills/career/fame/accessories/outfits, selects the party
   leader via `select_sim` (which safely preserves `data[81]` celebrity fame and
   `data[77]` date links across selection without hitting unported pools
   `0x446c15`/`0x44664c`), and establishes the mutual Downtown Date link
   (`person.data[77] = partner_id`) on `TravelParty::Date` trips.
5. **Atomic `TRVL` Envelope & Handle Authority:** Wraps the authoritative
   `PersistedTravelRecord { trip_id, is_date, view_centre, session,
   departure_snapshots, frozen_home_bytes }` in a `TRVL` v1 envelope stored in
   `NEIGHBORHOOD_LOTS[(nh, home_house)]` and commits it atomically with
   `staged_stored` (`secondary_house = destination_house`) via
   `save_lot_and_neighborhood_checked`:
   - Ordinary `lifecycle::enter_lot` on `home_house` is rejected while away
     because `Saved::decode` rejects the `TRVL` prefix, preventing accidental
     overwrites of the frozen home lot.
   - Live destination handles receive `neighborhood_origin = None` (blocking
     ordinary `lifecycle::save_and_leave_lot` from overwriting the Build/Buy
     destination snapshot) and `loaded = trip_binding_tag(store_id, trip_id)`
     (cryptographically binding the handle to its unique departure `trip_id`).

### 3. Return Home, Concurrent-Write Reconciliation & Crash Recovery (`return_from_destination`, `inspect_active_trip`, `resume_active_trip`, `abort_active_trip`)

1. **Trip-ID Fencing & Concurrent-Write Tolerance:** `return_from_destination`
   verifies `store_id`, `instance_id`, `destination_lot.loaded ==
   trip_binding_tag(store_id, session.trip_id)`, and `persisted.trip_id ==
   session.trip_id` against the REDB `TRVL` envelope (rejecting retained
   sessions from earlier trips to the same lot and rejecting already-returned
   handles). Because fencing keys on `(instance_id, trip_id)` rather than
   freezing `departure_revision`, unrelated neighborhood writes made while away
   (such as editing and saving another unoccupied destination lot) do **not**
   strand the active trip.
2. **Flush, Extract & Selective Neighborhood Merge:** Flushes every live Sim on
   `destination_lot` into `NBRS`, revokes temporary Vacation check-in hotel
   keys, captures returning traveler states (preferring each traveler's recorded
   `destination_object_id` and copying `accessories` + outfits + motives +
   `person.data`), and merges destination updates onto the **latest**
   `StoredNeighborhood` (updating touched/new neighbors, their bidirectional
   relationships and inventories, `TATT`, `made_characters`, and
   `session.home_family`'s budget/friends/`secondary_house = 0` while preserving
   unrelated families and untouched off-lot neighbors).
3. **Home Reconciliation & Single-Transaction Commit:** Restores the frozen home
   lot directly from the authoritative `persisted.frozen_home_bytes` (taking
   exact `ticks`, `world.ticks`, and `globals[0/5/6]` clock from the decoded
   `Saved` snapshot rather than caller-supplied session fields), keeps
   non-traveling Sims/pets untouched, deduplicates and resets returning
   household travelers cleanly at the home curb/portal, selects the returning
   leader via `select_sim`, and commits the unwrapped `Saved` home lot + merged
   `StoredNeighborhood` in a single atomic checked REDB transaction fenced on
   the latest `(instance_id, revision)`. Clears `destination_lot.loaded` so the
   handle cannot be returned twice.
4. **Crash / Restart / Halted-Destination Recovery:**
   - `inspect_active_trip`: reads and validates any active `TRVL` envelope for
     `home_house`.
   - `resume_active_trip`: reboots the destination lot from the persisted
     departure snapshots after a crash/restart or halted lot, atomically
     rotating `trip_id` in the `TRVL` envelope so any stale pre-crash handle is
     invalidated.
   - `abort_active_trip`: settles an active or halted trip directly from the
     persisted frozen home snapshot without needing a live destination handle,
     revoking temporary hotel keys, clearing `secondary_house = 0`, and
     unwrapping `TRVL` back into a normal `Saved` lot under a checked commit.

## Known VM Follow-Ups (Outside Disjoint Scope)

While ticking live Complete Collection destination lots, three unported VM
features outside this PR's disjoint `travel.rs` scope were observed:
- `Find Best Interaction` on community/Vacation (`zoning == 2 || 40..50`) and
  unzoned (`zoning == 0`, Downtown/Studio Town) lots (`autonomy.rs:434-445`).
- `Generic Sims Call` routine `26` on Magic Town community lots
  (`primitives/generic.rs:104`).
- Superstar Studio Town memory scope `38` (`Trap::UnsupportedScope { scope: 38 }`)
  on Studio Town lots (`House81.iff`).

Home lots (`House05.iff`) and Magic Town residential lots (`House90.iff`,
`zoning == 1`) tick all 220+ ticks in strict `Lot::step()` with zero errors.
Community/unzoned destination lots (`House21`, `House40`, `House58`, `House81`,
`House93`) tick all 220+ ticks cleanly under `Lot::step_play()` with every
caught object fault verified to satisfy `Trap::unported()` (`Trap::Unsupported`,
`Trap::UnsupportedScope`, or `Trap::Unimplemented` — zero `Trap::Invalid`, zero
`Trap::Native`, zero `Trap::BudgetExhausted`, zero corrupted slots/objects).

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
