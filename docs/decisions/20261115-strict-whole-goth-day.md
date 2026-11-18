# The day turns, and strict mode runs the Goth house's whole day

*Decided 2026-11-15.*

**Unverified past tick 2510, and wrong from tick 2515.** Strict mode on
the Goth house now runs 43,200 updates, a whole day from the load, to
06:30:44 on the next day (17 June 1997, one day played; hash bc43e887…)
with no stop. The state at every pinned tick up to 9906 is the same:
snapshot format 29 appends `World::stops_asked` (below), and the
format-28 hashes kept beside the new ones match.

Track 2's Goth day recording (`goth-day-20261115`) was still being made
when this was written; it held updates 1 to 14,759. Against it, our
random streams and scheduler cursor equal the original's after every
update to 2747, and our full state equals its saved full states (every
tenth update) to 2510, with two exceptions. The head target's last bit
differs, as already known (`20261114-head-follows-target-k-366.md`).
Motive 13 also differs: person 16's at updates 1060 and 1070, and person
18's at 2110 and 2120, read 61.757 where the original has 41.418. It is
equal again ten updates later and moves nothing else; it is not
explained yet. In tick 2515 they part: person 21 gets up from
chair 174 (facing 2) through Snap mode 3 to the chair's routing slot 1,
whose offset is (0, -16). We put the person at (424, 392); the original
puts them at (440, 376). It is the first time in the day that Snap
places by a slot's offset that is not zero (0x454b93), so that path had
never been compared. The lot's random stream then parts at update 2748.
Everything here from tick 2515 on is therefore read from the original's
code but runs on a state that already differs from the original's. The
fix is being worked on separately.

Two play-mode runs got further too, also unverified:

- The snack order (Have A Snack at tick 51) no longer stops at step
  9907. It stops at step 13199, where object 18 (tree 0x101c node 15)
  runs Burn (opcode 9), which is not ported. Strict mode, without the
  order, does not reach a fire.
- The snack cancelled on the way (steps 150 and 825) no longer stops at
  the route stop (0x45b3eb): the route stops at the walk's next step.
  Cancelled at 825 he stops on the tile's centre at (296, 456), puts the
  snack down in front of him at step 894, as when he is cancelled eating
  it, and is free at step 913.

**Ported on the way, in the order the run reached them:**

- **Tick 9907: Set To Next mode 11**, the neighborhood's other lots
  nearest first (0x44f08d). The original walks a vector of lot numbers
  (neighborhood +0x11c) that the lot load builds from a table of lot
  positions (+0x110, 0x428041). Both are empty in every capture of the
  original, and a write watch from the neighborhood view to the Goth
  lot's first update saw nothing write the table: the lot load only
  clears the vector (0x428110). So the vector is taken as empty and mode
  11 always takes the false branch. Reopen if a capture shows +0x110
  filled.
- **Tick 10719: a dynamic group placed** (0x4235ee, 0x4234ab,
  0x4234d8). A tile of a group whose type has callback function 8
  (flood.iff's puddles) is made by Create New Object Instance. Then each
  tile of its group works out a mask of the eight tiles around it that
  hold a tile of its own group, and runs its function 8 with that mask.
- **Tick 11267: a route's first run with its stack object gone**
  (0x45adb5) takes the false branch before a context is made.
- **Tick 12727: a planner search that reaches no goal** (0x482b35)
  forgets the chosen candidate (0x481745), and the context's result is
  3 (no way).
- **Tick 13655: a person looking at itself** (head target its own ID):
  the point on top of its own head (0x59293d on its own animator).
- **Tick 13757: a route to a door that one of its routes goes through**
  (0x45b176) fails.
- **Tick 14673: Drop Onto** (opcode 43, 0x44e96a) puts what I hold in
  one of my container slots into a container slot of the stack object.
  Main's port (#212) came in first and is the one kept.
- **Tick 15188: Snap's wall test**, flag bit 2 (0x45522c). My footprint
  at the object, facing the direction (0x433d16), crossed by a wall on
  its level (0x476dbc), sets word 54 to 0, and I am not put there.
- **Tick 15296: the selected Sim coming onto the lot** (0x4475d2) takes
  the lot's first indicator into its head slot. Whatever the head slot
  held is let go first (0x4481eb).
- **Ticks 16800 and 16828: a dynamic tile goes alone** (0x42346a,
  0x442def). Find Location For mode 1, or a removal, of a dynamic
  group's tile first takes the tile out of its group's chain (0x4228d6).
  Then the group it left and the tile work out their neighbours again.
- **Tick 18788: Snap modes 1 and 2**, into a container slot of the stack
  object (0x4549c7). Main's port (#214) came in first and is the one
  kept; the wall test and the make-way request below are added to it.
- **Tick 25500: a route asked to stop while under way** (0x589e2d,
  0x45b3eb) does not stop at once. The request is kept (animator +0x284,
  `World::stops_asked`), and the walk stops the route at its next step
  (0x58eeeb). A route's start clears it (0x589d38). **#188 ports the
  same step**: whichever merges second takes the other's state and drops
  its own.
- **Tick 26409: a layer below weight 1 blends** with what the bones held
  (0x598226 to 0x5985f9), where the bone blends. A layer at weight 0 or
  below is not sampled (0x597c4c).
- **Tick 29128: scope 24 field 5**, a neighbor record's own friend count
  (0x427148). While the count is out of date it is worked out again from
  the records of the `Characters` types (kind 1) with a family, friends
  both ways at the autonomy tuning's threshold.
- **Tick 29493: a route that ends where its person does not fit** puts
  it back on its own tile (0x589dee), with result 0.
- **Tick 30320: Create's person test is left out** (0x44e6b6 to
  0x44e727). Object 243 makes person 36, whose person words 84 (+0x634),
  68 (+0x614) or 74 (+0x620) have the tested bits set. The original then
  calls 0x45810c with 1 and sets a screen flag (0x4ceceb). 0x45810c
  looks the type's handle up in a table at 0x6f2698 that only the
  interface code reads, resets seven members of its entry (0x4e895e,
  0x5a5f79) and sets the same flag. Nothing the simulation reads is
  written, so the port does nothing. **Unverified** that the table is
  only what is drawn. A recording where a person is made with person
  word 84, 68 or 74 set would settle it; it is on track 2's capture
  wishlist.
- **Tick 30321: a person who goes through everything**, bit 0 of person
  word 84 or 68 (0x482413). It plans in its own room whatever the slot's
  (0x4824a0), with no objects' obstacles (0x48258f) and no room
  rectangles or walls (0x4827e3), and with the width limit (0x4828ba).
- **Tick 31478: the day turns** (0x46d3e9; `daily.rs`). When the hours
  reach 24:
  - global 28 (days played) and global 1 (the day) go up and the hours
    go back by 24;
  - every object on the lot's list has its daily update, age then value
    (0x442628, 0x432243);
  - the budget's days move back one;
  - the month and year roll over at 30 days (0x473544 always returns
    30);
  - signal 0xdc is sent instead of 0xdb. The lot's handler (0x41637f)
    moves the usage counts back a day and works out the lot's statistics
    again (0x416085, global 22).
  - the same statistics pass works out the objects' value again
    (simulator +0xa0, `Budget::objects_value`, 0x416118). Each object it
    does not pass over adds its value (0x43219a) when it stands on the lot
    and is valued (0x4354d5). It is not valued with flag 0x10 in data
    word 43, or when it is not a person and its catalog master's word 69
    is set. A group counts its lead's value, and the value is halved when
    data word 15 is set on any tile. **Checked against the save:** worked
    out at the Goth house's day turn before the objects lose value, the
    sum is 14660, the value the original saved with the house. After the
    loss it is 12524.

  **Not ported: the architecture's value** (+0xa4, 0x416126). It is the
  objects that are not valued but whose master's word 69 is neither 0
  nor 4, plus what 0x47b006 returns from the walls' style prices
  (0x6e5838), the wall and floor patterns' prices (0x4c63cf, 0x4a1439)
  and the pool's (0x6c5ff8). The pattern catalogs hold no prices, so the
  value keeps what the house was saved with (15277 for the Goths).
  Nothing ported reads it, so a run is not affected, but a save written
  after a day turns would hold a stale value. Failing loudly here would
  stop every lot at its first midnight, so the gap is documented instead.
  Reopen when the catalogs carry prices.

  The neighborhood's daily record of the household (0x429f14) is left
  out: it appends to a list of the family's that nothing ported reads.
  Of the other subscribers, the simulator's (0x46d2c2) and the
  architecture's (0x47b22a) have no case for 0xdc, nor do 0x43c0e4 and
  0x44302e. The interface's three subscribers are presentation and were
  not read in full.
- **Tick 34719: Snap asks a blocking person to make way** (0x45bd5e,
  from 0x454e54). With flag bit 1, a place another person is in asks
  that person to move, as a route does (0x45bf2e, `in_way.rs`).
- **Tick 35102: Set Balloon/Headline kind 7 mode 1**, a neighbor's
  picture (0x44b862). The picture is there when the stack object names a
  neighbor record whose type (+8) is in the catalog. This replaces the
  "not ported" of kind 7 in
  `20261115-newbie-house-tick-562-generic-sims-call-2.md`.
- **Tick 37426: Create New Object Instance mode 9**, the tile in front
  of me (0x44e157). It is one step in the direction my facing plus a
  local names, refused when a wall closes that tile toward me. Here it
  makes a puddle beside another, and **a dynamic tile joins the group of
  one of its type beside it** (0x423368). Its group is chained after the
  other's last tile, and every tile takes the lead's facing and its
  offset from the lead, turned back by that facing.
- **Tick 40935: an action that goes in before queued ones drops them**
  (0x45d4c8). The actions from that place on are dropped, and each
  target that names an object runs its function 4 (cancelled) with the
  person as its stack object. The new action then goes after the kept
  ones.

Every port has a synthetic test. Reopen any of these when the
comparison with the Goth day recording disagrees.
