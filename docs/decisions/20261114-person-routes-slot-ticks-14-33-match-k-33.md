# A person routes to a slot, and ticks 14 to 33 match (K = 33)

*Decided 2026-11-14.*

In tick 14 of the Goth house, person 259 goes to a routing slot of
object 51 (Go To Routing Slot, primitive 45). That route is now ported:
the route context, the places to stand, the planner and the animator's
start of the route, its following of the path and its turn on the spot.
The end of a clip in Animate Sim is ported too (tick 24). Our state
after each of ticks 1 to 33 equals the capture. K goes from 13 to 33.

Some terms. A *route context* is the record a routing primitive makes
for a person and pushes on the person's list (person +0xaf0); it holds
the slot, the places the person could stand (*candidates*) and the one
chosen. The *path* is the list of points the planner gives (person
+0xae0). The *animator* is the person's object that plays clips and
moves the person along the path; its *state* says what it is doing (3
standing, 4 turning on the spot, 9 following the path). A *clip* is one
animation; the *foreground slot* holds the clip a script started.

What was decided:

- **Go To Routing Slot is ported for the route the house takes**
  (`crates/sims-vm/src/primitives/go_to_slot.rs`). Its steps run from
  the frame's primitive state, as in 0x45ad63: make the context
  (0x48163e), find the candidates (0x48235a), plan (0x45c38e), start
  the route (0x589c85), then wait while the route is under way. Every
  other step and result fails loudly. The slot's index is the operand
  word at byte 0. The word at byte 2 says where the index comes from;
  only 1, a literal, is ported.
- **The routing state is part of the lot** (`sims_sim::route::Routing`,
  `World::routing`): each person's contexts and path, the slot flags
  that differ from the type's, and the person words the original wrote
  from memory it never set.
- **Person word 37 is held as unknown.** The planner copies a
  candidate's word +0xe into it (0x45c418), and the slot search never
  writes that word. The original's value is whatever its memory held
  (3287 in the capture). We store 0 and mark the word unknown; a script
  that reads it fails loudly, and a write makes it known.
- **The planner is ported** (`routing/planner.rs`, `regions.rs`,
  `rooms.rs`, `search.rs`), from the original's code and the old
  prototype's search (legacy/crates/sims-sim/src/lot_route/search.rs,
  legacy/crates/sims-sim/src/region_path.rs,
  legacy/crates/sims-sim/src/regions.rs). An object's obstacle keeps its
  footprint's words in their own order. An obstacle that costs 8 or
  more, or is a person, grows by 3. Only an outdoor room on the ground
  of at least 100 tiles is ported.
- **The animator follows a path and turns on the spot**
  (`crates/sims-vm/src/animator.rs`): state 3 (0x58ddbf), the start of a
  turn of more than 3 eighths the negative way (0x58df01), state 4
  (0x58e045) and state 9 (0x58e4e0, 0x58eb29). A new animator and a reset
  one have segment -1 (0x587d2c, 0x589220). Angles use the FPU's
  `FPATAN`, which `sims-core` gives as `Ext::atan2` through libm.
- **A clip's end is ported** (`primitives/animate.rs`, 0x589c25): the
  animator stops playing, takes its state from the posture (0x592904:
  0 gives 3, 1 gives 1, 2 gives 2), and brings its base layer up to
  date (0x590430). The pickup and person word 19 are cleared, and the
  true branch is taken. Starting the next clip clears the finished one
  from the foreground slot at once (0x586b63). The original fades it out
  instead when byte 0x6f6bd0 is set; that byte is 0 in a dump of the
  running game, and the fade is not ported.
- **Snapshot format 13** adds the route fields of each animator and the
  routing state. Every older pin in `tests/resize.rs` and
  `sims-sim/tests/world.rs` still holds once the new fields are taken
  out (`format_12_bytes`), so the pinned states are the same.
- **The harness can dump the game's memory** (`tools/harness/qmp.py`,
  `capture.py --dump`). QEMU's `memsave` reads the stopped guest; nothing
  writes it. The dumps feed an emulator of the original's code that is
  outside the repository.

How it was checked:

- Our state after each of ticks 1 to 33 was compared with the capture's
  after the same lot update, field by field, floats bit for bit. The
  fields are those of [20261114-script-can-make-person-ticks-6-13-match-k-13](20261114-script-can-make-person-ticks-6-13-match-k-13.md), plus the animators' route
  fields and each person's number of route contexts. The list is in the
  doc comment of `the_goth_house_first_tick_matches_the_original`.
- The capture does not hold the context or the path. A new run of the
  harness dumped the original's memory at the entry of updates 14, 15,
  47 and 580. The original's update was run in an emulator on the dump
  of update 14. Its context, the planner's obstacles, room rectangles
  and goals, both searches, and the 11-point path equal ours.
- Ticks 1 to 13 still match, and the load comparison with its trace
  gives the same 3 lines as before.

What did not match, and why:

- After tick 1, global 12 and registry byte 0x12. These are the known
  differences of [20261114-script-can-make-person-ticks-6-13-match-k-13](20261114-script-can-make-person-ticks-6-13-match-k-13.md) and the entries before it.
- After tick 34, person 259's animator. The capture has 0 as its
  direction and its destination's direction; ours has the turn's (0.127
  and 0.210). The emulated update writes the same direction as ours, at
  0x58488f, so the 0 is written outside the lot update. The dump of
  update 47 also has 0.
- From tick 36, registry byte 0x12 (ours 1, the capture 0). Its cause
  was not looked into.
- Tick 46 does not finish. Person 18 starts a clip with timed events
  (0x5912fd), which is not ported.

What is **unverified**:

- What writes 0 into person 259's animator direction after tick 34. A
  guess is the interface: an animator's position can be unset (virtual
  function 0x20, 0x589361) and placed again from the person (virtual
  function 0x60, 0x592132). Nothing ported runs the interface.
- That `Ext::atan2` gives the FPU's value. libm is correct to within one
  unit in a double's last place; the game rounds to 24 bits, so only an
  angle within about 2^-52 of a rounding midpoint could differ.
- That the sound events of a footstep change nothing the simulation
  keeps. They are not modeled.
- The branches of the routing the house does not reach. They fail
  loudly. The synthetic tests check the obstacle rule, the following of
  a path and the turn against our reading of the code.

What would reopen this: a difference found in ticks 14 to 33, or the
writer of the animator's direction after tick 34.
