# Same-value object data stores, the lot 22 Go Home, fuzzer pick targets, and FAMh export

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-vm/src/{memory,caf,travel}.rs`, `crates/sims-vm/src/primitives/{random,set_to_next}.rs`
  - `crates/sims-vm/tests/integration/{caf,fuzz}.rs`, `crates/sims-vm/tests/interpreter/{memory,random,set_to_next}.rs`
  - `crates/sims-sim/src/{neighborhood_store,lib}.rs`
- **Track:** Track 2

## Context

#452 found that after a trip on from Downtown lot 21 to lot 22, the cab sign's Go Home asked for no trip home within 3,000 frames. Its test fell back to `return_from_destination`.

The strict tick fuzzer of #449 failed at seeds 0x64 and 0x6c (error 0x34) and 0x6b (error 0x0a).

`export_family_fam` wrote no FAMh, though the original's exports carry one (`20261125-famh-move-in-wiring-and-creation-depth-guard.md`, "Not done").

## Decision 1: storing the value a guarded object data word already holds succeeds

The VM refuses an Expression store into object data words 2 and 3 (the container and its slot) and 11 (the object's ID). The original stores them like any other word: 0x453ff1 (scope 3, my data) and 0x454051 (scope 4, stack data, special-casing only word 25) resolve the word to `object + 0x4a + 2 * index`, and the assign at 0x452e8b stores through it. The refusal stays for a changed value, since the original keeps the container's slot table and its table of objects by these words, and a changed value would leave those behind.

Storing the value the word already holds changes nothing, so it now succeeds. Lot 22 holds eight objects whose room tree (tree 4102, first seen on object 387) first sets their own ID to their stack object, themselves.

Two primitives check their destination by storing its own value back before they work out the value they store. That check now passes for these words, so each handles them itself:
- **Random Number** checks every other destination as before, and then draws from a copy of the stream as that check left it (a store into Global 3 under co-op can run a callback that draws). For words 2, 3 and 11 it stores the drawn value, which stores nothing or faults with no other effect, and only then keeps the copy. A draw of the value the word holds is stored, and any other faults with the stream unchanged (`interpreter::random::a_draw_of_the_value_an_object_word_holds_is_stored`).
- **Set To Next** refuses the three words before any mode runs, as before. Mode 4 would otherwise list the instances of a type before the store of the next instance faults (`interpreter::set_to_next::an_object_s_container_slot_and_id_words_are_refused_before_any_mode`).

What the refusal cost on lot 22, in play mode:
- Each such store faulted, which reset the object and left the scheduler's pass partial.
- `world.ticks`, which advances only on a full pass, advanced 13 times in 25 frames, with about 223 faults on each of those objects.
- The animator clock and every script waiting on ticks ran at about half speed.

The cab sign's Go Home on lot 22 therefore asked for the trip home after 3,793 frames. With the fix it asks after 2,110 frames, close to lot 21's 2,182.

`travel::tests::go_home_from_a_second_downtown_lot_asks_for_the_trip_home_in_time` replays the trip on from lot 21. It checks that 400 frames advance `world.ticks` by 400 with no script errors, and that Go Home (action 0x107b) asks for the trip home to house 5, for the leader, within 3,000 frames. It fails without the fix. `interpreter::memory::object_data` checks words 2, 3 and 11 in my and the stack object's data: a changed value is still refused, and the same value is stored with both objects unchanged.

The Goth `TICK_43200` pin is unchanged.

## Decision 2: the #452 test's dialog answers are a harness artifact (Track 4 follow-up)

The second cause is in the `sims-game` test, which this track does not own. The shell opens the lot with the scripted answers `[3]`. `Answers::choices` keeps an answer until a dialog consumes it, so the leftover `3` answers the cab dialog with its second button. That button is "No", the false branch, so no trip is asked for.

Clearing the answers on lot 22 before Go Home (`game.lot_mut().replay_dialogs(&[])`) makes the test pass without the `return_from_destination` fallback. This was checked locally and reverted. Track 4 should make that change in `crates/sims-game/tests/integration/trip_requests_and_neighborhood_iff.rs`.

## Decision 3: the fuzzer picks only from objects the lot draws

The three strict failures are harness artifacts.

The fuzzer drew pie-menu targets from every object in the world, including the 151 objects of the House05 lot parked off the lot at [-16, -16]. Those are the factory's and removal's: the helpers of the lot's people and interactions, and the off-lot people. The lot never draws them, so no player can click them.
- At seeds 0x64 and 0x6c, the pick is on parked object 306 (kind 7, whose private trees include Praise and Scold). Scold's node 2 sets the stack object to stack attribute 1, which is 0 when the menu is run that way, giving error 0x34. A plain, non-co-op `Lot::pick` for Sims 16 and 18 faults the same way, so it is not the co-op layer's.
- At seed 0x6b (reproducing only at 40 steps), the pick is with caller 259 on parked object 269.

`draw_step` now leaves out objects at [-16, -16]. Contained objects keep their real positions, so they stay candidates. With that change, the three seeds and a strict sweep of 32 seeds from 0x64 all pass. The module doc says so.

Possible follow-up: the co-op executor accepts a pick on an off-lot object from a peer. The original's UI cannot send one, so a peer that does is misbehaving. Refusing it would be a protocol decision, left open here.

## Decision 4: a Family Bin export writes the family's FAMh

The Family Bin export writes the family through the same routine as the `Neighborhood.iff` save (0x410696, called at 0x428a17). That routine writes FAMI and FAMs, then the family's FAMh, taken from its `.FAM` file (+0x140, vtable +0x74 at 0x4107a8) or else from its record (+0x144). After it, the export writes CTSS #1000 (0x428a6c), EXPi (0x428ae9) and the members, NBRS and FINV. A family with neither file nor record writes no FAMh.

The store's save inputs already hold the FAMh the save writes for each family (`SaveInputs::histories`). So:
- `caf::export_stored_family_fam(store, neighborhood_id, family_id, family_name, language, include_finv)` reads the neighborhood, its character files and its save inputs from one REDB read transaction (`NeighborhoodStore::load_export_snapshot`, returning `sims_sim::ExportSnapshot`). It writes the family's FAMh when there is one.
- `caf::export_family_fam_with_history` takes the payload directly.
- The FAMh chunk has the family's ID, flags 0x10 and an empty label, and comes after FAMs and before CTSS #1000.
- `export_family_fam` keeps its signature and writes none, as for a family with no FAMh (Create-a-Family's).
- `load_character_files` and `load_lot_snapshot` share the new `character_files_in`.

`caf::a_stored_family_exports_the_famh_its_fam_brought_byte_for_byte` checks five things:
- A family imported from a `.FAM` with a FAMh, under a new ID, exports that payload byte for byte, with the family's new ID.
- The chunk order is FAMI, FAMs, FAMh, CTSS #1000, EXPi.
- The export is otherwise the one without the FAMh.
- The Create-a-Family family, and the family whose file had none, export none.
- Re-importing the export saves and exports the same payload.

`caf::a_moved_in_family_still_exports_its_famh_after_a_lot_save` (with `SIMS_INSTALL`) moves such a family into House04, plays 50 ticks and saves the lot. Its export still carries the payload byte for byte.

## Consequences

- Objects whose scripts restate their own container, slot or ID no longer fault. Lot 22's scheduler runs a full pass every frame.
- The fuzzer's strict runs no longer pick from objects parked off the lot, which no player can click. Hidden objects on the lot stay candidates.
- An exported `.FAM` carries its family's FAMh like the original's.
- Not done:
  - the `sims-game` test change of Decision 2 (Track 4);
  - no shell path exports a `.FAM` yet (only `sims-game` tests call `export_family_fam`). A Family Bin Export button should call `export_stored_family_fam`.
