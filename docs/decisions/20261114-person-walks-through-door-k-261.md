# A person walks through a door, and K = 261

*Decided 2026-11-14.*

In tick 233 of the Goth house, person 21 routes to an object in another
room. Routing between rooms through doors is now ported, with what the
walk reaches next: slots searched by distance, the end of a path, and a
room noting who is in it. Our state after every one of ticks 1 to 261
equals a new fourth capture of the original, which holds the full state
after every update to 608. Tick 262 stops loudly. K goes from 232 to
261. Why the first capture chose a tick later than the others is not
known, and cannot be told from what can be recorded: this is a stop
condition for modeling it.

Terms used here. A *portal* is a door seen as a way from one room to
another. A *capture* is a run of the original under the harness that
saved its state. A *frame* is one drawing of the house view, as far as
it writes the simulation's state.

What was decided:

- **Routing between rooms is ported** (`routing/portals.rs`). People and
  doors take numbers from one pool (object data word 6, 0x46381a; reset
  at 0x4638b9). Each door holds a vector of costs (object +0x1ac), one
  column per number. A door's column holds its distance to every other
  door through rooms and doors (0x46390f). A person's column holds the
  cost of reaching the target through each door of its room (0x463c83:
  10001 for no way, 10000 for a door the route may not use, 1101 for a
  door that failed, 1001 to 1100 for one tried before, the distance
  otherwise). Step 2 of the route (0x45afd0) takes the cheapest door,
  notes the try (0x463e43) and pushes the door's tree on the person's
  thread. A second try, or a door tree that returns false, marks the
  door failed (0x463e09).
- **Slots are searched by distance** (0x483598). Each place is
  classified (0x481439): off the lot, another room, closed off by walls
  or an object (0x4359ea), uneven ground, or free. A place is turned by
  the slot's direction (0x483d4b). The last step of a route to a slot
  takes the slot's facing (0x45c516 to 0x45c5fb).
- **The planner plans indoors and upstairs** (a free rectangle costs 0
  there, 0x4824d0 to 0x48253e), and without the width limit in a small
  or crowded room. The ratio of objects to half tiles is rounded to
  single precision before it is compared with 0.2 in double precision
  (0x468103, 0x46810a), so exactly a fifth counts as crowded.
- **The end of a path is ported for a person with object data word 9
  set** (0x58e740). It walks on until it is near the end point, then
  stands. Standing with the route's end noted clears the walking flag
  (0x58f117). The route then ends (route_result 0x589d47, end_route
  0x45b6ea; a route under way does not stop, 0x589e2d). The person is
  put on its tile and turns along the last segment.
- **Rooms note who is in them** (0x468802). A person whose room changes
  tells both rooms. A dark room with no lamp then shows the light of a
  lit room (0x4688bf), so it waits to be counted again. A person's visit
  in the load notes its room too (0x45e43d, called at 0x459c22).
- **Each frame counts the rooms that wait** before it draws (0x4a8895,
  called at 0x4a8b03). So the frame schedule changes the rooms' state.
- **The default frames are `0,1,2,3,33+30`**: before tick 1, after
  ticks 1, 2 and 3, after tick 33, and after every 30th tick from there.
  The three captures that watched the draw guard all show this. The
  earlier default, a frame after every tick from 33 (`0,1,2,3,33+`),
  counted the rooms sooner. The `N+S` form is new in `sims boot
  --frames`.
- **Snapshot format 16** keeps each room's mark (Room +0x73) and the
  animator's last step (+0x294, +0x298), its near-end flag (+0x288) and
  its finishing flag (+0x28c). The pins taken under format 15 still
  hold with these left out (`format_15_bytes`). The states after ticks
  5, 13, 33 and 232 were pinned again, because the rooms now hold other
  values from the frame after tick 2. Everything the captures hold is
  the same before and after.
- **A third and a fourth capture were made.** The third holds 300
  updates (full state after 1 to 100 and every tenth, 26 MB). The fourth
  holds the full state after every update to 608 (130 MB). Both watched
  the draw guard. The harness now saves each door's portal costs as
  bit patterns (`portal_bits`). The fourth stopped at update 609: the
  harness found an object in a retained list that the registry did not
  hold (`ValueError` from `qemu_retained_owners.py`), and it refuses to
  guess. The captures stay private under the old checkout's `cache/`.

How it was checked:

- After every one of ticks 1 to 261, our state equals the fourth
  capture's, floats bit for bit: every field listed in
  `the_goth_house_first_tick_matches_the_original`, the people's
  animators (with the new fields), the route contexts and paths, all 71
  doors' portal costs, and the `updates.jsonl` line (tick count, both
  random streams, the C runtime stream, the scheduler cursor).
- The second capture is equal after every tick it holds (to 240). The
  third is equal but for the two lines below.
- `the_goth_house_first_tick_matches_the_original` runs to tick 261,
  pins the state there, and checks that tick 262 stops at 0x58e8be.
- The load comparison still gives its three known lines. Ticks 1 to 232
  still match.
- The new mechanics have synthetic tests: the pool and columns, the
  distance between doors, choosing and trying doors, closed directions,
  the turn, the slot order of refusals, the planner's two settings, the
  end of a path, and rooms noting people in a tick, a visit and a frame.

What did not match, and why:

- Global 12 after ticks 1 and 2 is 17 in the third capture (and the
  first), and 0 in ours, the second and the fourth. It is the
  interface's wait check, which we do not run.
- One local of object 18's seventh frame after ticks 210 and 220 is
  16104 in the third capture, and 0 in ours and in the fourth.
- At the stop in tick 262, person 21's person word 27 is 40 in ours and
  41 in the capture after the tick. The steps after the stop presumably
  write it.
- The first capture does not choose in tick 233 (below).

**The first capture's tick 234: not modeled, likely the harness's pause.** Its state after
tick 230 equals ours in all compared fields, and its streams are equal
to 232. After tick 240, person 21 runs tree 0x208a node 10 with no
running action: it chose a tick later. Ruled out:

- The frames. Fifteen schedules in our model all choose in tick 233.
- The dialogs. Notices can close without a press (one run showed it),
  so the presses are not the input.
- The cursor. It differs between runs, but runs with different cursors
  both chose in tick 233.
- The state. The original's code, run in an emulator from the first
  capture's own memory at update 47, chooses in tick 233.

What differs: only the first capture was stopped and taken over at
update 45 (its events: `attach resume=True updates=45`,
`took_over_at_return`), so the game stood still for a while. Object
+0x154 and +0x158 look like timestamps in milliseconds, and they differ
between runs. Find Best Interaction checks interface state before it
chooses (0x45ddc0 to 0x45dde2): the selected person (0x4e97dc),
[0x6e48bc]+0x10c, and byte 0x6f2a54 (written at 0x4ef21f). Any of these
may depend on timing. The captures do not hold what would tell, and
timing cannot be recorded as an input with this harness. So it is not
modeled; we follow the other three captures, which agree with each
other and with ours. This is not a roadmap stop condition: the original
is observable, and three runs that were not paused for long behave the
same. The likely cause is the long pause at update 45, which a normal
run does not have; that is **unverified**. An experiment that pauses a
run on purpose would test it. If the original's choices turn out to
depend on wall-clock time in normal play too, a fixed time input will
be needed, as for the frames.

Found wrong along the way:

- `draw_frame` did not count the waiting rooms, though the doc in
  `rooms.rs` said frames did. Without the count, motive 13 of a person
  differed from the fourth capture after tick 260.
- The planner's doc said "at most a fifth"; the single-precision
  rounding makes exactly a fifth too many.
- The slot's three proximity words were named minimum, optimal and
  maximum. The search (0x483598) reads them as the least, greatest and
  best distance, in that order.
- The walking flag was not cleared when a person stood at the end of a
  route, and a visit did not note its room.
- Route context word +0x68 had no meaning in our docs. It holds the
  door the route goes through.
- A doc comment about cells in `placement/find.rs` sat on the wrong
  function.
- The last entry's claim that a frame after every tick from 33 gives
  the same state after tick 232 no longer holds for the rooms (it still
  holds for what the captures hold). Its default `0,1,2,3,33+` is
  replaced.

**Unverified:** that route context word +0x74, which the original leaves
as the heap held it, is never read; which of the candidates above made
the first capture choose later; that nothing outside the compared fields
depends on the frames within updates 1 and 2; that the draw's other
writes do not matter; that the steps after the 262 stop write person
word 27.
