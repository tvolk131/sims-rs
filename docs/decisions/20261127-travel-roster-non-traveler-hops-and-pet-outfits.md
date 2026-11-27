# Travel: visited-lot roster changes, hops that take a non-traveler, pet and non-human clothing lists, deleted visitors' outfit rows

*Decided 2026-11-27.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-vm/src/travel.rs`
  - `crates/sims-vm/src/primitives/generic/outfit_editor.rs`
  - `crates/sims-vm/src/lifecycle.rs`, `crates/sims-vm/src/lifecycle/outfit_rows.rs`
  - `crates/sims-vm/tests/interpreter/outfit_editor.rs`
- **Track:** Wave parity: the open Travel & Outfit Edge Cases. These are follow-ups to #501 and #508.
- **Supersedes in part:** the follow-up list of [20261127-outfit-editor-29-30-31-outfit-rows-in-character-files.md](20261127-outfit-editor-29-30-31-outfit-rows-in-character-files.md). That ADR refused pet clothing and non-human clothing categories, and left deleted visitors' outfit rows in place.

## Context

#501 (travel PR 1) and #508 (outfit editor and rows) left four edge cases open:

1. A family-roster change on a destination lot never reached the neighborhood. The travel merge took the family's budget, friends and magic from the visited lot, but not its `members`. So if a member left the family during a visit (Generic Sims Call 6, Leave Family), a later family arrival or the home lot made that member again.
2. A trip on (`travel_on_from_request`) refused any party member who was not already a traveler.
3. Routines 24, 29 and 30 refused a pet's clothing categories (1 to 5), and routine 31 refused any non-human's.
4. A deleted visitor's edited outfit rows (`World::made_outfits`) stayed in a saved lot. A later character that reuses the GUID would read them.

## Decision 1: the visited lot's roster goes into the neighborhood

The native trip handler (0x4fc59c) saves the whole neighborhood from the live families (0x428843 → 0x4286a0) before it loads the next lot (0x4f1ae9). Leave Family (routine 6, 0x44fc53 → 0x42601e) does three things:
- removes the member from its family's roster;
- clears word 61 (FAMILY) on the person and on its record;
- marks the family stale.

So whatever reads the neighborhood next sees the roster without the member. That includes the next family arrival (0x4f2253, which makes every member) and the home lot.

The port now does the same:

- **Merge.** `merge_destination_into_latest_neighborhood` takes the household's `members` from the visited lot whenever they differ from the trip's baseline. This applies to both the same-revision path and the concurrent path; the concurrent path has already checked that the latest neighborhood's members equal the baseline's.
  The write-back drops a family left with no members, as 0x425efb does. So a home family that is missing from the visited lot's write-back counts as an empty roster.
- **Membership follows the merged roster.** After the merge, every traveler's `is_household_member` is recomputed from the merged roster, both on a trip home and on a trip on. The next leg's envelope, and so its abort and its trip home, sees who is a member now.
  - On a trip home, a former member does not return. It stays with the visited lot, as a traveler a hop leaves behind does, and its record stays in the neighborhood.
  - On a trip on, a former member can go on as the leader's date, as someone who is not a member.
  - The leader of a trip on must be a member in the lot's live roster.
  - The MagiCoins the return reports count the members of the merged roster.
- **Home reconcile.** The frozen home lot still holds a person for each member as it was at departure. The reconcile reads that roster from the frozen world's families, before the neighborhood's replace them. Each person who was a member then but is not one now is purged. This covers a member who left the family and came home with the trip, and one who stayed on an earlier lot. A stale word 61 plays no part. The purged person's outfit rows are kept, because its character still exists.
- **Members the home lot has no person for.** A Sim can join the family on the trip and then not come home with it, for example a date who joined while the leader went on alone. The home lot's load makes every member of the family (0x4f2253), so the reconcile makes such a member from its record and places it with the travelers. With no member coming home, the selection falls back to a member of the household, never a visitor.
- **Kind-3 tokens on the trip home.** A traveler who left the family does not come home as a member. So when the caller's kind-3 token names it, the token is kept and the Sim's saved state stays for routines 20 and 21. The home lot's routine 20 then finds the Sim or makes it, as it does for a companion who stayed behind.
- **No member left.** If every member leaves the family on the visited lot, the native lot load clears the empty family's house and removes the family (0x428071 → 0x425efb). The port does not port that cleanup. It refuses the trip (`InvalidParty`) before anything is written, because a housed family with no members would stop later lot loads.

Tests (install-backed, Goth on Magic Town lot 93):
- `a_member_who_leaves_the_family_on_a_trip_does_not_come_home`
- `a_family_arrival_does_not_remake_a_member_who_left_the_family`, which goes alone to 21, back to 93 in mode 1, then home.
- `a_former_member_goes_on_as_a_date_and_an_abort_leaves_it`
- `a_former_member_the_trip_home_asks_for_is_brought_home_by_routine_20`
- `a_member_who_joined_the_family_on_a_trip_is_made_at_home`
- `a_trip_home_whose_traveler_left_the_family_selects_a_member` (Downtown 21 alone)
- `a_trip_home_with_no_member_left_is_refused`

## Decision 2: a trip on may take a Sim who is not a traveler

0x4fc59c has no idea of travelers. It saves the neighborhood and loads the next lot, and who arrives there is read from the neighborhood:
- a family arrival makes every member of the family;
- on a date, the companion is the one the leader's kind-1 token names (routine 19, run only by Downtown's middle ped marker).

Routine 17 (0x45a22d) saves the leader and the companion, or every member of the family on the lot (mode 1, 0x45a3ef).

So `onward_request` no longer requires the party to be a subset of the travelers:

- Every party member who is not a traveler must have a neighbor record; otherwise the request is refused as `InvalidParty`.
- The check on routine 17's saved state now refuses only a saved neighbor that is neither a traveler nor a party member.
- The leader must still be a household member who is travelling.

Each joiner enters the trip the way a departure takes a companion who is not on its lot (`joining_snapshots`):
- **State:** from its person on the lot if there is one; otherwise from its record, with full motives.
- **Outfits:** its outfit rows from the lot.
- **Hotel key:** the hotel-key flag from its current tokens.
- **Household membership:** set when the home family lists it.
- **Home object:** none, because it did not come from the home lot.

After the merge, each joiner is checked against the neighborhood the trip writes. A joiner is refused if:
- its record was deleted while the lot was open, so its GUID is in `deleted_characters` or now has another record; or
- its `deleted_at_epoch` is later than the epoch the trip began at, which the frozen home lot records. This catches a later character that took the deleted Sim's GUID and its record's place.

Tests:
- `a_trip_on_takes_a_companion_who_did_not_travel`: Downtown 21 to 22, with a kind-1 token naming an off-lot outsider.
- `a_trip_on_with_a_companion_who_has_no_record_is_refused`.
- `a_trip_on_with_a_companion_deleted_meanwhile_is_refused`.
- `a_trip_on_with_a_companion_replaced_meanwhile_is_refused`.

## Decision 3: pet and non-human clothing lists

The clothing list builders (0x408d8e `f`, 0x408e7b `s`, 0x408f68 `l`, 0x409055 `w`, 0x409142 `h`) never check gender. Each fill (0x4098f1 for `F`, and the same code for the other letters) builds this pattern:

| step | native | adds |
| --- | --- | --- |
| letter | 0x472657 with 0x6c0fb0 (`F`), and so on | 1 |
| skip | 0x47275d(3) | 3 |
| sex | gender 0 → choice(`m`,`u`), gender 1 → choice(`f`,`u`), any other gender → nothing (0x409941..0x409966) | 1 or 0 |
| age | `c` if 0x40cbdd says child, else `a` | 1 |
| skip | 0x47275d(3) | 3 |

The match (0x4727f1) returns the matched length. A texture belongs to a suit when it starts with the suit's first matched-length characters followed by the skin tone (0x40d007: row 14's first three characters, lowercased, or `???`).

For a pet (genders 2 to 5) or an unknown gender (-1), the pattern is therefore 8 characters with the age letter at index 4. The port builds these lists. The refusals for pet clothing in routines 24, 29 and 30, and for non-human clothing in routine 31, are gone:

- **Routines 24 and 29** write the entry and keep the place worn. The commit's pet branch (0x40a5d5) then writes `NONE` over rows 2 and 30 to 34, as before.
- **Routine 30** answers the place worn, then commits `NONE` the same way.
- **Routine 31** counts the list, for a pet and for an unknown gender.

An unknown gender is still refused in routines 24, 29 and 30. Their commit reads the null body list (0x408bf5 → 0x40d0a4), which this change does not port.

Tests, all on a synthetic wardrobe:
- `generic_call_31_counts_a_pets_clothing_without_a_sex_step`
- `generic_call_30_answers_a_pets_clothing_worn_and_commits_none`
- `generic_calls_24_and_29_dress_a_pet_in_clothing_the_commit_takes_off`
- `generic_call_31_counts_clothing_for_an_unknown_gender`

## Decision 4: a deleted visitor's outfit rows leave the lot

A person's rows are that character's strings, which the original keeps in its definition and file (0x587bbd). When the character is deleted, they go with it.

Three places load a lot and purge orphaned or deleted people:
- `lifecycle::boot_from_saved_lot`;
- the saved destination boot in `travel.rs`;
- the home reconcile, `reconcile_frozen_home_lot`.

Each now also drops, from `world.made_outfits` and before any traveler's rows are added:
- the rows of every person it purges (except a household member who only left the family);
- the rows of every character deleted since the lot was saved, meaning it is listed in `deleted_characters` or its `deleted_at_epoch` is later than the lot's `deletion_epoch`.

The helper is `outfit_rows::drop_deleted_rows`. `reusing_deleted_guid_purges_stale_visitor_on_older_saved_lot` now also checks that the stale visitor's rows do not reach the character that reuses its GUID.

## Open item: the lot-session clothing-list cache

Native does not rebuild a list for each wearer. It keeps the lists in a process-wide vector, 0x6e4890:
- each record is 0x90 bytes, keyed by skin tone (compared by 0x46f874) and by the slot `child * 2 + (gender != 0)`;
- 0x40951c clears it, called from 0x4e8bcf through the vtable method 0x4ef9d3, which is the lot teardown.

So within one lot session, a pet shares a slot with a light-skinned adult woman. In the install, pets have skin `lgt` or `unk` and age `27`. Whichever of the two builds a list first decides that list for both.

The port builds each wearer's own list. That matches native on a cold fill. It differs once another wearer in the same slot has filled the cache first: for example, a light-skinned adult woman's formalwear list is then the list a light-skinned adult pet counts and dresses from.

Being faithful needs the cache as lot state: a `World` field that is saved with the lot and kept in step for co-op. That means `sims-sim/world.rs`, the save format (`SNAPSHOT_FORMAT`) and `coop.rs`, all outside this change's files. A cache kept outside the world would make co-op peers diverge, so this stays open until a track that owns those files can take it, with tests for both fill orders.

## Verification

- `TICK_43200`, `GOTH_DAY_STREAMS` and every other strict pin are unchanged.
- `cargo test --release -p sims-vm -p sims-sim` passes, with `SIMS_INSTALL` and `SIMS_UD2_AFTER` set.
