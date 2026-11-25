# Household groups arrive through the marker; routine 17's tokens go there and back; lot shift and 0x467436 scope

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-vm/src/{travel,selection,routing}.rs`, `crates/sims-vm/src/routing/rooms.rs`
  - `crates/sims-vm/tests/integration/travel.rs`
- **Track:** Track 2

## Context

The roadmap's travel follow-ups listed three items:
- `HouseholdGroup` and Magic Town 90–92 trips can still snap back (#414);
- routine 17's kind-2, 7 and 8 tokens (saved motives, the arrival clock) are neither written nor read (#414);
- the lot shift (0x444991) should forget routes, and 0x467436 at go-live is not modeled (#443).

## Decision 1: a household group is left off the lot wherever a marker places it

**The snap-back.** The cause was found with a probe that steps each trip 660 ticks and logs every step of more than 24 units.

A `HouseholdGroup` trip (Mortimer and Cassandra) to Magic Town lot 93 is `ArrivalFamily::Group`. Its travelers were put beside the arrival portal, where they took up an action and started walking. On tick 21 the middle ped marker (PedMarkersMagic tree 0x100e) runs Generic Sims Call 26. That call gives temp 1 = 1, since the game holds the home family (global 9 = 5). The marker then takes its family branch:
- it loops over the people (Set To Next 1) whose word 61 is the family;
- it sets their word 34;
- it puts them beside marker 0x13076784 with Find Location;
- it runs routine 21 on each.

Both travelers were mid-walk. Mortimer's put was undone in the same tick: he stood at [289, 225], back on his path, not by the marker. Cassandra was put at [440, 344], and her walk took her back to [281, 216] on tick 25. This is the jump the Family arrival had before #414 (`sims-game` `magic_town_arrival`).

**The fix.** Group travelers are made off the lot at [-16, -16], as the person factory leaves them (0x4e9719 → 0x440b2d), when both of these hold:
- 0x4f1ae9 makes an arrival on the lot (`has_native_arrival`; for a household group, which goes only to Magic Town, that means lots 93–99);
- the loaded lot holds Magic Town's middle ped marker (GUID 0x12e84823) on the lot.

The marker then places each of them once, as Family arrivals already work, so `HouseholdGroup` needs no native mode.

Otherwise nothing would place them, and they are put beside the arrival portal as before:
- Magic Town's residential lots 90–92;
- a lot whose marker was sold with `move_objects` (Sol found this one: without the check, both travelers stayed off the lot for 300 ticks).

Solo and Family travelers still wait for the marker on every lot. On a lot whose marker was sold they would stay off the lot, as nothing else in the original places them either. That is not checked against a capture.

**Not handled: a marker that runs but places nobody.** Sol found this in round 2. If lot 93's cab stop (0x13076784) is moved upstairs to [200, 200, 2] with `move_objects`, the marker's Find Location (mode 0 beside the cab stop, then mode 5) fails. The marker still clears its attribute 3, and its travelers stay at [-16, -16].
- This happens on departure and on resume, with v5 and v3 records alike.
- It is not specific to household groups. A whole-family trip to the same edited lot, which takes the native arrival path untouched here, leaves all three travelers at [-16, -16] after 300 ticks. The marker's script is the original's, so the original most likely does the same, though no capture checks it.
- A port-only rescue would apply to every marker arrival, so it is left as a follow-up rather than special-cased for groups. One option: once the marker clears attribute 3, put any traveler still off the lot beside the portal.

`HouseholdGroup` is still `Group`:
- the travel plan accepts it only for Magic Town;
- no departure tokens are written, so the travelers keep their live motives. Routine 21 still runs on each, as it did before this change. It restores only kind-2 tokens the traveler's inventory already held, which a departure never leaves (the return and the request cancel erase them), so normally it restores nothing;
- the port makes only the travelers.

**Lots 90–92 do not snap back.** Several trips to these lots were stepped 660 ticks, and none jumped:
- whole family to 90;
- single to 91;
- household group to 90.

The roadmap's "90–92 can snap back" comes from the same Group path. On these lots no marker moves anyone, so there was no snap-back.

**Test.** `travel::a_magic_town_household_group_is_placed_once_by_the_marker`:
- **Lot 93:** each traveler starts at [-16, -16] and makes exactly one jump, the marker's placement onto the lot within 30 ticks, and none in the 300 ticks watched.
- **Lot 90:** the group starts on the lot and never jumps.
- **Resume:** `resume_active_trip` arrives the same way on both lots, and the trip home succeeds.

On lot 93, the marker's placement must land within 3 tiles of the cab stop (0x13076784) that its Find Location targets. A put that a walk undid within the same tick would therefore not pass as a placement. Without the fix the test fails: on lot 93 a traveler starts at [280, 232].

`travel::a_household_group_to_a_lot_without_its_marker_is_put_by_the_portal` sells lot 93's marker in edit mode (with `move_objects`), saves, and sends the group. The travelers start beside the portal and never jump. Without the marker check, they start off the lot and the test fails.

## Decision 2: routine 17's saved-state tokens already round-trip; a test now proves it

The roadmap item is stale. Since #426 and #430, `depart_to_destination` runs routine 17 (`Machine::request_trip`, 0x45a22d) for every Solo and Family arrival. That writes the kind-2 tokens:
- GUIDs 0–6: motives 9, 6, 5, 15, 7, 8 and 14, cut toward zero (`__ftol`). A motive cut to 0 gets no token (0x41f38d).
- GUIDs 9–12: person words 8, 80, 82 and 81.
- GUIDs 7 and 8: the arrival clock (0x45a785), in member 0's inventory.

"Kind 2, 7 and 8" means these kind-2 tokens with GUIDs 7 and 8; there are no token kinds 7 or 8. Routine 21 (0x45098a) restores the saved state. Routine 23 (0x450b2b) reads the clock back, but only in the phone plugins' tails on the home lot, into locals nothing reads (`20261124-travel-polish-and-caf-modes.md`).

`travel::tests::saved_motives_and_the_arrival_clock_go_there_and_back` takes the Goth family (three members) to Magic Town lot 93. Each member's seven saved motives get distinct values with a fraction, some negative; Cassandra's motive 5 is 0.6, which cuts to 0. The test spells out the GUID-to-motive table itself, apart from `trip::SAVED_STATE`, so a wrong table fails it. It checks:
1. **Departure.** Each member's kind-2 tokens, in the world and in the stored neighborhood, are exactly the cut motives in routine 17's order, with no token for 0.6. Member 0's tokens end with the clock tokens (`arrival_clock` of the home clock and Magic Town's arrival time).
2. **Arrival.** Once the marker has placed everyone (tick 21), each saved motive equals its token's count exactly. The motive with no token keeps, within 1, the value the traveler was made with. Every motive token is consumed. Member 0's clock tokens stay, since they name nothing for routine 21 (0x450a53).
3. **Return.** After 30 more ticks, `return_from_destination` brings each member home with the motives they had on the destination. No kind-2 token is left, in the world or in the store, clock included.

Nothing changed in the snapshot or envelope formats (`TRAVEL_ENVELOPE_VERSION` stays 5). Group trips still write no tokens: no native routine 17 asks for a household group, and their travelers carry their live motives directly.

## Decision 3: the lot shift and 0x467436 need no port

**Lot shift (0x444991).**
- It has one caller, 0x47be52 in 0x47be21, which then also shifts the terrain (0x47c292).
- 0x47be21 has one caller: the HouseViewer message handler 0x47b22a, for message 0x106 (0x47b2d9).
- The one `push 0x106` in the image posts it from the developer cheat `offset_world [x_amt] [y_amt]` (0x4a557e; usage string at 0x6c7178). It refuses unless the view is at rotation 0 (0x6c719c).
- 0x444991 first forgets the routes (0x444805). It then walks the objects and offsets each one's position. Depending on whether 0x422632 finds the new place on the lot, it calls either 0x44123f with the object's ID or 0x433e50 with the new place.
- Neither the VM nor the shell has `offset_world`. No path in the port moves a lot, so no cached route can outlive one. A future port of the cheat must call `forget_routes` first, as 0x444991 does. `routing.rs` says so.

**0x467436 at go-live.**
- 0x416534 calls 0x467436, then 0x466c01 (marking the rooms), 0x444805 and 0x416085.
- 0x467436 walks the rooms manager's rooms. For each room whose +0x34 is set, it empties the room's kept planner rectangles: 20-byte entries at room +0x28 to +0x2c, removed by 0x468761 → 0x4689f8.
- The next use of a room's rectangles (0x46795a) finds them empty and works them out again (0x482347).
- The port keeps no rectangles: `routing/rooms.rs` works them out on every use. After go-live the original does the same, so the two already agree.
- The `go_live` and `rooms.rs` comments now say so, replacing "Not modeled".
- `routing::tests::going_live_forgets_the_routes` still covers the part of go-live that has state.

## Consequences

- A household group to a Magic Town community lot is placed once by the marker, every traveler included, and stays where it is put.
- The Goth `TICK_43200` pin is unchanged. No home lot or Goth path runs this code.
- Still open:
  - a family arrival makes only the members who traveled (#414). Household groups rely on it to make only the travelers, and still will if Family is changed to make every member, since they are `Group`;
  - a date's companion is still made by the departure;
  - the hop that leaves travelers behind (#443);
  - the outfit-editor routines (#430).
