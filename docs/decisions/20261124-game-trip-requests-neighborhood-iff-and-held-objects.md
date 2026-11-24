# 2026-11-24: Trip requests and Neighborhood.iff in the game shell; held objects checked

## Context

#435 and #436 landed VM APIs that `sims-game` did not call yet:

- `NeighborhoodStore::save_neighborhood_iff` (#435);
- `Lot::cancel_trip_request`, `depart_from_request_with` and
  `return_from_request` (#436).

The shell still declined a trip by marking it in `Game` only. It departed
with the destination's default transport and answers `[3]`, and it refused
every trip a destination's script asked for. Nothing wrote
`Neighborhood.iff`.

The roadmap also listed "carried objects drawn at the Sim's hand
(`0x459dc0`)". That landed in #413
(`20261123-person-held-objects-at-attach-points.md`). This change checks it
over real play and adds the tests that were missing.

## Trips a script asks for

Generic Sims Call 17 sets `World::trip_request`. The original's main loop
finds it between ticks and always leaves the house (`0x4fc713`).
`NeighborhoodShell::pump_trip_request` now does this:

- **From home** (the phone's Go Downtown, the Magic Hole, ...):
  `depart_from_request_with`, with:
  - the transport of the lot picker the script asked with
    (`picker_transport`: Magic Hole, Magic Blimp, Star Limo), used only
    when the pick names the requested lot;
  - the shell's own dialog answers and frame schedule, as for a trip it
    plans itself.

  The arrival places the travelers by the transport.
- **Home from a destination** (Call Cab/Go Home: routine 17 with house
  -1, which `0x45a22d` resolves to the family's house):
  `return_from_request`. This is the old `return_from_destination` body,
  now a private `return_home(requested)`.
- **Refused**, with a notice:
  - co-op;
  - a trip from one destination to another (Go Somewhere Else, tree
    `0x1016`);
  - editing a destination;
  - a house that is not a destination;
  - a departure or return the VM refuses (for example
    `IncompatibleTransport`).

  The request is then declined.

`Game::decline_trip` now cancels through `Lot::cancel_trip_request`:
- The request is dropped.
- The kind-2 saved-state tokens and the arrival clock routine 17 wrote are
  taken back, so the script's tail on this lot restores nothing from them.
- A later routine 17 asks again instead of adding to a stale request.

This is a deviation, since the original has no refusal. A house opened
alone and a headless still decline the same way.

**In co-op** the world changes only through the lockstep. So `decline_trip`
still only latches the request (`declined_trip` with `same_trip`), as
before. The shell refuses travel in co-op anyway.

## Neighborhood.iff

The original saves the neighborhood (`cTSNeighborhood::Save`, `0x4252cc`)
from 23 call sites. These include:
- the lot save (`0x428687`, at `0x428819`);
- eviction (`0x42737e`, at `0x4273e2`);
- the trip (`0x45a818`);
- the family screens.

The shell now writes the open neighborhood's committed record through
`NeighborhoodStore::save_neighborhood_iff` (`NTemp.iff`, then renamed over
`Neighborhood.iff`) whenever a commit has moved the record's
`(instance_id, revision)`. That covers:
- a lot save;
- entering a lot (its reconcile commit);
- a departure or return;
- a co-op stage;
- every `refresh_from_store` (family moves, evictions, CAF).

Details:
- **Where.** `<saves>.userdata/<neighborhood id>/Neighborhood.iff`
  (`NeighborhoodShell::neighborhood_iff_path`), beside the saves file and
  its `options.iff`. The install is never written.
- **One write per revision.** The shell remembers the record it wrote, and
  opening a neighborhood counts as written. Opening alone writes nothing.
- **REDB stays the store.** A failed write is logged and does not undo or
  fail the commit. The next commit tries again.
- **One writer per folder.** This is #435's own assumption. Each saves file
  has its own folder.

## Held objects at the hand

#413 draws a Sim's slot 0, 2, 3 and 4 contents at attach points 0, 3, 1
and 0, which `sims_people::attach` binds from the body strings. Three
findings:

- **The off-viewport residual in the #413 ADR does not happen.**
  `People::refresh_poses` does not cull by viewport, only off-lot Sims and
  floors above the camera's. So every Sim the held pass can draw objects
  for has its points. What such a Sim holds is on the same level, and the
  held pass draws only levels up to the camera's.
- **Open-pack mode has no points.** Its Sims are 2D sprites with no
  skeleton. What they hold stays at its own position, as before. The
  open-pack Sims are not the original's, so there is nothing to match.
- **Still not ported** (unchanged from #413):
  - The original draws held objects before their Sim (`0x459e45`, then the
    Sim at `0x45a131`). Ours draws them after all people. Both passes are
    depth-tested. Only a translucent edge differs.
  - Slot 1 animated objects (`0x58f8ff..0x58fa91`).
  - Pets.

  Porting the draw order would move the people pass in `sims-people` and
  `sims-render`, and every ratchet frame with a carried object. It is left
  for a change of its own.

`Game::held_attach` returns the points the last render's held pass used.

## Verification

New install-gated integration tests in `crates/sims-game/tests/`:

- `trip_requests_and_neighborhood_iff::a_lot_save_writes_neighborhood_iff_beside_the_saves`
  - Opening writes nothing.
  - A lot save replaces a stale file.
  - The file imports into a fresh store with the household's house and
    members.
  - No `NTemp.iff` is left.
- `trip_requests_and_neighborhood_iff::a_destination_go_home_returns_through_the_shell`
  - A date Downtown, then the cab sign:
    - Go Somewhere Else to lot 22 is refused with the notice. The lot
      drops the request and plays on, and the trip is unchanged.
    - Go Home asks for house 5 and the shell returns.
  - The saved-state tokens are gone on the lot, in the store and in the
    file.
  - The file follows the family: secondary house 21, then 0.
- `phone_go_downtown::a_declined_trip_lets_the_lot_play_on`: declining
  returns the request, the lot no longer has it, the saved state routine 17
  wrote is taken back, and the lot plays on.
- `magic_town_arrival`: the jumper's trip goes by `MagicHole`, the picker's
  transport, not Magic Town's default blimp.

For a solo trip or a date Downtown, `return_from_destination` already
erases the leader's departure tokens. So the Go Home test also passes with
that return. Only a group trip, or a kind-3 companion token, tells the two
apart, and the VM's own tests cover both
(`a_trip_home_from_a_group_trip_takes_back_its_own_tokens`,
`a_trip_home_a_script_asks_for_takes_back_its_tokens`). No Magic Town lot
offers a Go Home to test it through the shell.

Held objects:

- `goth::what_the_sims_carry_is_never_drawn_on_the_floor`: Mortimer's snack,
  then free will for 6,000 ticks, rendered every 10 ticks. 44 held
  object-frames over objects 27, 28 and 263. Each had its holder's point
  for the slot.
- `sims-people` `a_held_object_is_placed_on_the_hand_that_holds_it`: at
  every zoom, rotation and level, the renderer's moved-object placement of
  `View::lot_point(hand)` lands within one fine unit of the hand vertex the
  person draws. That fine unit is the whole fine position `0x4b9991`
  places at.
