# Generic Sims Call routine 19: bring back a traveler's companion

*Decided 2026-11-23.*

Downtown's middle ped marker (EP3 `PedMarkers.iff`, GUID 0xbc17335c,
tree 0x100e) runs its arrival pass once a family arrives:

- it puts the family's first member beside it (node 3) and clears its
  attribute 3 (node 10);
- it runs Generic Sims Call 19 for that Sim (node 11).
- **If routine 19 names a companion** (temp 0 of 0 or more), the marker
  places the companion too and sets up the date: DateFlag, a headline, a
  greeting.
- **If it names nobody**, the marker sets the Sim's PetsOutside word
  (node 24).
- Either way it waits 30 ticks, restores saved state with routine 21 and
  takes down the travel picture with routine 3.

Routine 19 was not ported. A headless trip from the Goth house to
Downtown lot 21 showed its fault at node 11, after which the marker's tree
was reset. Attribute 3 was already clear, so unlike Magic Town the marker
did not place the Sim again. But nothing after node 11 ran: no companion,
no PetsOutside, and no routine 21 or 3.

What was decided:

- **Routine 19 is ported from 0x450787.**
  - The running object's temp 0 names a person (native error 0x1c
    otherwise). The person's neighbor inventory is found, or added empty
    (0x42a8a1).
  - The first token of kind 1, any GUID, is found (0x41f69c), and its
    place is kept as a word.
    - A negative place sets temp 0 to -1 and ends the routine. That covers
      none found, and an index past 0x7fff that wraps negative. Nothing is
      erased and nothing is saved (0x450cde).
  - The token's GUID names a person type. Temp 0 becomes the object ID of
    the lot's first person of that type, or -1:
    - a type missing from the interface's table of person types
      (0x4e9753) gives -1;
    - otherwise the lot's people are walked (0x4e9608);
    - with nobody of that type, the type's definition goes to the object
      factory (0x4e9719 → 0x440b2d) and the walk runs again.
  - One is taken from the token's count, and the token is erased when none
    is left (0x41f56b with 1).
  - Kind-1 tokens are what Manage Inventory's operation 5 writes: the GUID
    of the neighbor a traveler takes along.
- **The neighborhood save at the end (0x4252cc) is not modeled.** The
  neighborhood's inventories live in the world and are written when the
  lot is saved. They differ from the original's file only if the game
  stops before the next save.
- **A headless Downtown date leaves the companion's token.**
  `TravelRequest::downtown_date` stands in for the date's departure tree,
  which runs Manage Inventory operation 5 on the leader before the trip.
  Departure now does what that operation does (0x455437) to the leader's
  inventory: every kind-1 token is erased and one is added with count 1
  and the companion's GUID. Without it the marker took the no-companion
  branch on a date. An aborted date (Abort Trip, our recovery, not the
  original's) takes the token back, as it takes back a hotel key the trip
  granted; otherwise the leader's next trip would bring the companion.
  The travel envelope's version goes to 2 to say a departure wrote the
  token. A version 1 trip, saved before, is still read, and its abort
  leaves the leader's tokens alone: a token there was the leader's own.
- **The residual scan reads character files as a lot's load does.** It
  adds the Characters folder of each neighborhood (UserData, UserData2 to
  UserData8) with `Index::add_characters` (found whatever its case,
  subfolders and archives included), so a tree in `Characters/Imported/`
  or a character archive is scanned too. A Characters entry that is no
  folder fails the scan rather than being skipped.
- **Routine 19 comes off the residuals.** `RESIDUALS` in
  `crates/sims-vm/tests/generic_routines.rs` now lists 23 routines.

How it was checked:

- Six interpreter tests in `crates/sims-vm/tests/interpreter/neighbors.rs`
  cover:
  - a companion already on the lot;
  - the count running down;
  - no token;
  - a type that is no person;
  - a companion the factory makes;
  - a temp 0 that names no person;
  - places past 0x7fff.
- `crates/sims-game/tests/downtown_arrival.rs` sends the Goth house's
  selected Sim to Downtown lot 21. It checks that:
  - the marker's arrival pass runs past node 11 (the Sim's PetsOutside
    becomes 1);
  - no error from the marker is caught.

  Without the port it fails on the routine 19 fault.
- A second test there sends the same Sim on a date with an adult of
  another family. It checks that routine 19 took the leader's kind-1
  token, and that the marker marked the companion (PetsOutside 1) and set
  its DateFlag to the leader. Without the departure token it fails on the
  companion's PetsOutside.
- A third test departs on a date, reopens the neighborhood, aborts the
  trip and checks the leader holds no kind-1 token. A unit test in
  `travel.rs` rewrites such a trip as version 1 and checks its abort keeps
  the token.
- The Goth house's 43,200-tick hash is unchanged.

**Unverified:**

- The date's own departure tree. The token is written by the headless
  date departure, not by the tree that writes it in play: the phone's
  "Call Cab.../Go Downtown" action is dropped from the queue before it
  runs. Whether the original also places the companion at arrival, as
  the headless date does, or leaves it to routine 19 to make, is part of
  that follow-up.
- **The marker takes the family's first member as the traveler.** It
  finds the arriving Sim with Generic Sims Call 18, which names the first
  member of the game's family (0x4e97dc(0)). On Downtown our family is
  still the whole household, Mortimer first. So a trip without Mortimer
  is never placed: a solo trip by Bella leaves her where she arrives,
  PetsOutside stays 0 and the marker goes on waiting at node 2. A trip
  with Mortimer on it is placed with Mortimer as the lead, even one Bella
  leads with him as her companion; routine 19 then reads Mortimer's
  inventory, not hers. The original lets any adult go Downtown, so it must
  give the destination a family the traveler leads; which rule it uses is
  not yet found. The date test uses the household's first member for
  this reason. That is a travel follow-up.
- The traveler jumps back about 16 ticks after the marker places it. The
  Sim's walk keeps its own position (see the Magic Town note,
  `20261123-generic-routines-21-and-3.md`). That is the arrival-hold
  follow-up.

What would reopen this: a run of the original where routine 19 picks a
different person, makes one where this port does not, or leaves the
inventory differently.
