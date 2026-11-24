# Day-turn architecture value, forgetting routes, a trip's companion and its next destination

*Decided 2026-11-25.*

These are four `sims-vm` parity fixes. Addresses are in the original
`Sims.exe` 1.91. This decision closes both deviations listed in
[20261124-travel-polish-and-caf-modes](20261124-travel-polish-and-caf-modes.md).

## The architecture's value at the day turn

The day turn's lot-statistics pass (0x445100) sums two subtotals:

- the objects' value, stored at simulator +0xa0 (0x416118);
- the architecture's value, stored at +0xa4 (0x416126). This is the
  value of every object that does not count toward the objects' value
  but whose catalog master's word 69 is neither 0 nor 4 (0x445190 to
  0x445223). Added to it (0x416121) is what 0x47b006 returns, which is
  the walls', wall patterns', floors' and pool's prices.

Before this change the port stored only +0xa0. +0xa4 kept the value the
house was saved with, so the Furnishings rating and the next save used a
stale architecture value. `daily::store_values` now stores both. It uses
the same `lifecycle::architecture_value_and_built` that the load runs.
The lot's prices travel on the interpreter
(`Interpreter::architecture_prices`, set by `boot` and `coop` lots). An
interpreter without prices leaves +0xa4 alone. The travel module's
private copies of the pricing are gone.

0x47b006's built flag (global 22) now counts walls. A tile with wall
segments (0x4795c5, 0x47b08c) counts as built even with no floor or
pool. Before, the strict pass failed loudly on a lot like that, and the
resilient pass guessed.

Evidence from the Goth-day recording (`update-NNNN/state.json.gz`,
`simulator_storage`):

- Native +0xa4 is 15277 from load until update 31477.
- At the day turn, update 31478, it falls to 13777. The 1500 is exactly
  the one architecture object that the recording removes before update
  25000.
- So the original recomputes +0xa4 at the day turn and not before.
- Our run keeps that object, so 15277 is right for us after the turn.
  The `TICK_43200` pin (`df6eb544…`) and every other Goth pin are
  unchanged.

The boot test now notes the recording's drop at 31478.

## Forgetting routes (0x444805)

0x444805 walks the lot's people (lot +0x84). For each one, 0x45eddc
clears byte +0x78 of every route context. Then 0x46387c fills each
portal's cost vector (+0x1ac) with 10001 (0x667478) and keeps the
vector's length. The port calls this pair `Interpreter::forget_routes`
(`routing.rs`) and `forget_portal_costs` (`routing/portals.rs`).

Callers:

- **Go live (0x416534).** `selection::go_live` now calls it after
  marking the rooms. 0x467436 is still not modeled.
- **A door moved.** The portal class's vtable (0x667480) overrides
  take-off with slot 20 (0x4634f2, base 0x422d86) and put with slot 22
  (0x4630d9, base 0x422c2f, then 0x463159 and 0x476a18). Both end in
  0x46387c. 0x44099b gives the class to a multipart object whose master
  has type 8 (0x440a91). `placement::door_moved` runs it at the end of
  `take_off` and `put_as` for such objects.
- **Lot shift (0x444991, from 0x47be52).** Not ported. The port has no
  lot shift.

The Go To Slot/Routing state machine resumes a context whose byte +0x78
is clear (0x45aea5), as 0x45b9d4's jump table does:

- State 2 (through a portal) clears +0x68.
- State 3 (under way) goes to state 15.
- In both cases the byte is then set again.

State 15 (0x45b3aa) asks the animator to stop (0x589e2d). It yields
until the animator stops, then plans again from state 1 (0x45b05a).
Before this change the resume was `Unsupported`.

The Goth save's portal costs are already 10001, so no pin moves.

## The trip home makes a companion who stayed behind

On the home lot, the Go Downtown plugin's tail (n14 and n90) and the
Vacation plugin's tail (n79/n81 and n89) run routine 20 (0x45087d) on
the caller. It finds or makes the person the caller's first kind-3
token names, and takes one from the token. When that names a person,
the tail runs routine 21 (0x45098a) on the companion, which restores it
from its kind-2 tokens.

`return_from_request` placed the travelers from their live state. It
took the token on the destination lot and did not make a companion who
had not travelled. Now, when the token names someone who is not a
traveler:

- that companion's saved-state tokens are left alone;
- once the home lot is reconciled, `bring_companion_home` runs routines
  20 and 21 there, as the caller, on the lot's stream;
- the caller's temps are put back afterwards;
- routine 20 saves the neighborhood itself (0x4252cc), so the
  inventories the tail wrote go into the record the return commits
  (`store_inventory`).

A traveling companion keeps the old path.

## Destination to destination (Go Somewhere Else)

Go Somewhere Else (tree 0x1016) runs routine 17 with the picked lot and
0, 30, mode 0. 0x4fc713 only records the request (bytes +0x130 to
+0x138). The main loop's handler (0x4fc59c) does not tell a destination
lot from home:

1. It clears the request.
2. It sets the arrival flags (+0x140 to +0x142) from the house range
   and the recorded GUID or family.
3. It saves the neighborhood (0x428843 → 0x428041).
4. It loads the new house with 0x4f1ae9(house, -1), whose arrival marker
   restores the travelers from the tokens routine 17 wrote.

The family's own house stays as it was saved when it left.

The new `travel_on_from_request(install, store, session, lot,
&TripDeparture)` returns a `DepartureResult`. It is additive; the
existing functions keep their signatures. It works in three steps:

1. **It leaves the lot as a trip home would.** People are flushed into
   the neighborhood, temporary hotel keys are revoked, the lot itself
   is not saved, and the visit is merged three ways. The only
   difference: the saved-state tokens routine 17 just wrote stay, for
   the next arrival marker.
2. **It boots the next destination as a departure a script asked for
   would.** The shell supplies transport, answers, frames and camera.
   The tokens are already written.
3. **It rewrites the trip's envelope** and commits it with the
   neighborhood in one fenced transaction. The family's secondary house
   becomes the new destination. The envelope keeps:
   - the frozen home lot and home clock;
   - each traveler's home object.

   Its funds and MagiCoins on arrival become the ones the first
   destination was left with, which that lot's merge committed. The
   MagiCoins are counted on the booted lot, from the merged
   neighborhood. So a
   resume or abort starts from them, and the trip home reports the
   change since the hop.

   It gets a new trip ID, so the old destination handle and session
   stop working. Its baseline is the merged neighborhood, and its
   departure snapshots are the travelers as they left, leader first.

What is refused:

- A request for the trip's home house belongs to `return_from_request`,
  which still refuses another house.
- A request for the lot the trip is on.
- A house that is not a destination.
- A date to anywhere but Downtown. Only Downtown's middle ped marker
  (EP3 PedMarkers tree 0x100e) runs routine 19, which brings the
  companion in. Studio Town's and Magic Town's arrival trees do not,
  and a lot with no arrival marker has none. A date whose companion is
  the leader is refused too.
- A mode 1 request that saved someone who is not a traveler, such as a
  household member visiting the lot.
- A party that is not exactly the trip's travelers, all of them on the
  lot. For example, one member of a whole family asks in mode 0, so the
  others would be left behind. The original has no refusal: the members
  left behind stay in the neighborhood and turn up wherever the next
  load puts them. The port's session has nowhere to keep them. This is
  the one remaining deviation.

A refused request stays for the shell to cancel.

## Tests

- `daily::tests`:
  - the day turn works out the architecture value again;
  - walls alone count the lot as built.
- `routing::tests`:
  - going live forgets the routes;
  - moving a door forgets the portal costs.
- Interpreter seat tests for the 0x45aea5 resume, through a portal and
  under way.
- Travel:
  - `a_trip_home_makes_a_companion_that_stayed_behind_at_home`;
  - `a_trip_goes_on_from_one_destination_to_another`. It goes Downtown
    → Old Town → home, checks the refusals, the spent first session, the
    secondary house and the frozen home clock.
