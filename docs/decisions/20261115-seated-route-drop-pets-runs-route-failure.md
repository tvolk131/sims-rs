# A seated route, Drop, pets, runs and the route-failure feedback; strict mode to tick 9906

*Decided 2026-11-15.*

**Unverified.** The Goth day capture (`goth-day-20261115`, past update
8699) covers these ticks, but this run has not been compared with it.
One word was read from it: at update 6756 the new pet's bottom frame
(Main) has the arguments -2, 0, 0, 0, the -2 being the maker's temp 0
that Create flag 0x10 copies (below); before the fix this port left 0
there. Strict mode now runs through tick 9906 (12:00:56, hash
c8240fc7…) and stops at tick 9907, where object 5 (tree 0x1005 node 5)
runs Set To Next mode 11. The snack order stops on the same at step
9907. Snapshot format 27 appends the neighborhood's data words
(`World::neighborhood_data`); every pinned state up to tick 3912 is the
same state, which the format-26 hashes kept beside the new ones show.
The Newbie house, which stopped at tick 1803 on the same Create flag
0x4, now runs to tick 4052 (its tick-1802 state unchanged under format
26); the frame drawn after tick 4053 finds person 140's skin waiting
for its update (0x590d26).

**Ported on the way, in the order the run reached them:**

- **A seated person's route** (tick 3913, 0x4832d4, 0x45b1d7). A person
  in a container with a Sit function starts its route from the best
  place beside the seat (the seat's entry bits, definition word 66). If
  it already sits in a seat a candidate names (0x48177b) it is there;
  otherwise its seat's Stand tree runs first (step 5, 0x45b6d4) and, if
  that returns true with the person standing, the candidates are found
  again (0x45b439); otherwise the route gives up with result 6.
- **Neighborhood data** (scope 34, 0x4531bf): words 1 to 15 of the
  neighborhood's sixteen, read only, from the NGBH chunk. Others are
  native error 0x3c.
- **Create New Object Instance modes 2 and 5, and flag 0x10.** Mode 2
  (0x44e22f) puts the new object in the running object's first
  container slot, mode 5 (0x44e3e9) where the running object stands.
  Flag 0x10 without flag 0x2 (0x44e8e1 to 0x44e90c) writes my temp 0
  (+0x3a) to the first data word of the new object's bottom frame: its
  first argument, or its first local if it takes no argument; a bottom
  frame with neither fails loudly. With 0x2 it is native error 0x36
  (0x44e88c), and 0x2 is not ported.
- **Drop** (opcode 5, 0x454430): what my first container slot holds
  goes one tile in front of me, after its place test. #183 ported it
  too ([20261115-drop-ported-second-order-waits-press-cancels](20261115-drop-ported-second-order-waits-press-cancels.md)); this branch's port is kept, as #183's added
  the step's words to the wrong coordinates. 0x42228a's first word is
  added to +0xf4 (0x4544ab), which is our second coordinate, as in
  `routing::slot`. In #183's cancelled-snack test Mortimer stands at
  (296, 456) facing 6, so the snack goes to (280, 456), not (296, 440).
  **Unverified**: no capture covers a Drop.
- **A person's reset ends its object uses first** (0x45e814): frames
  are popped down to each use's count, and the object used resets.
- **The route-failure feedback tree** (step 17, 0x45af2a): this branch
  had ported it too; #181's port ([20261115-route-that-gives-up-near-slot-runs-failure](20261115-route-that-gives-up-near-slot-runs-failure.md)), checked against the
  Newbie recording, is kept in place of it.
- **Pets** (tick 6756). The person factory's pet guard (0x440d63) makes
  a "dog" or "kat" only with bit 0x20 (Unleashed) or 0x80 (Makin'
  Magic) of the application's word +0x66; `Program::expansions` now
  holds the installed mask the lot's zoning already used. The Complete
  Collection has both. From tick 6760 the pet searches for places with
  seats off (0x4823d0: person word 65 above 1, so the sitting weight is
  0 at 0x4835f8); with data word 17 (its pace) set the planner's free
  cost is 0 (0x482505) and a level-1 outdoor room's fixed rectangles
  leave out terrain (0x4827f6); and it turns by a sixteenth of a step
  over the rate, not a quarter (0x58df67). Dropping the seats of a slot
  with a sitting weight for a pet (0x46de84) is still not ported.
- **Running** (tick 6784, 8215). With pace 1 a walk starts in animator
  state 11 (0x58ed52), state 6 with the stop-from-run clip 0x16, which
  ends in state 10 (0x58e377), and the look-ahead uses the run clip's
  stride (+0x1fc) when the pace is not 0 (0x58e6cb).
- **Manage Inventory operation 0** (tick 7702, 0x4556f4): adds 1, or
  temp `(flags >> 2) & 3` with flag 0x2, to the first token of the kind
  and GUID (capped at 0x7fff), or adds a token at the end unless the
  inventory holds 0x7fff.
- **No door** (tick 9337, 0x45b036): a route that must go through a
  door and has none gives up with result 2.

**Set To Next mode 11** (stop at tick 9907, 0x44f08d) walks the
vector of lot numbers at neighborhood +0x11c: cursor 0 gives the
first, otherwise the one after the cursor; none, or the last, is the
false branch. The lot load (0x428041, at 0x428350) builds it from lots
1 to the length of the 8-byte (x, y) table at neighborhood +0x110 less
one, leaving out the current lot, sorted (0x42ffce) by squared distance
(0x427fb9). What fills the +0x110 table was not found:
`LotLocations.iff` (STR# rows of "lot, x, y") is read only by a
single-lot lookup for the interface (0x4e1f3a) and written by the
editors (0x54d8b7, 0x555b46), and the zoning vector (+0x174) is a
different one. Until that table is found, mode 11 fails loudly.
