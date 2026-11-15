# Find Location For is ported, and ticks 2 to 5 match (K = 5)

*Decided 2026-11-14.*

Find Location For (primitive 16) is ported for the modes the Goth house
reaches. With it, tick 2 runs to its end, and ticks 2 to 5 run. Our
state after each of ticks 1 to 5 equals a private capture of the
original after the same tick, in everything compared, the C runtime's
random stream aside. K is now 5. Tick 6 is beyond the capture and stops
loudly.

Some terms. The *place test* is the check whether an object may stand
at a position (virtual function 0x54, 0x433311). The *search* is the
lot's walk over tiles in a square spiral out from a start tile, looking
for one that passes its tests and the place test (0x4347f0). The *C
runtime stream* is the random number generator of the C library
(`rand`), apart from the game's own two streams.

What was decided:

- **Find Location For** (0x44b3c8, `primitives/find_location.rs`).
  Mode 0, and modes 6 and up, which the original treats the same: the
  search starts from tile (1, 1) on level 1, or, with flag 1, from the
  object a local names, and looks first in the start tile's
  room, then anywhere. Flag 2 allows a tile that already holds an
  object; flag 4 refuses tiles with ground flag 0x20. The tile found is
  tested once more, the object is put there with the lot told (0x433340)
  and the true branch is taken. Modes 1 to 5 fail loudly; mode 5 is the
  one that draws random numbers. Native errors 0x15, 0x33 and 0x17 are
  kept.
- **The search and the place test** (`placement/find.rs`), written from
  the original's code: the spiral (0x434ba9), the search's own tile tests
  (a diagonal wall, an empty grid cell, the same room, a pool tile, the
  ground flag), and the place test through 0x4329c1: the border and
  level, the walls the object needs or cannot stand against (0x476f5a,
  only for an object with no wall rules), water, pool, slope, floor,
  tile flag 4, the objects on the tile and the eight around it (0x4337f6)
  and the walls the footprint would cross (0x476dbc). A person, a tile of
  a group, a container, the upper level's floor test (0x4330b7), wall
  rules and two objects that are both not solid fail loudly. The old
  prototype's ports (legacy/crates/sims-sim/src/placement.rs `Spiral`,
  `search_tile_allowed`; legacy/crates/sims-sim/src/lot_acceptance.rs)
  were read alongside; they were checked only against the original's
  code run alone, which is weaker evidence than the capture.
- **Clip names ignore case when nothing matches exactly.** The script of
  people 18 and 21 names the clip "...-Rstand"; the animation library
  has only "...-rstand". The library's lookup (0x5a27f2) compares bytes
  exactly, but the capture shows the original playing "...-rstand" in
  tick 3, with the duration and frame count of that clip. Where the
  original ignores case was not found in its code. `Animations::clip`
  takes an exact match first, then the single name that differs only in
  case, and fails when there are two.
- **Sample names lose their directory.** Members of the animation
  archives whose names carry a directory are now stored under the bare
  file name, which is how scripts and the skeleton files name them. Tick
  3 needed this to find its clip's samples.
- **Create New Object Instance mode 8** (0x44e0c8, the jump table at
  0x44e92e, `primitives/create.rs`) is ported for an ordinary object,
  with no flag or only flag 0x20: the object is made where the object a
  local names stands, tested with the place test, removed again if it
  does not fit, and put there. Turning it (flag 0x20, when its facing
  differs) is not ported and fails loudly, as do a new person and an
  object of a group. This is only reached in tick 6, so it is
  **unverified**.
- **Object 15's dialog** in tick 2 is answered with 3, the second
  button, as the capture's events show. `sims boot` needs `--answers 3`
  to get past it.

How it was checked:

- Each of ticks 2 to 5 was compared with the capture's state at the
  return of the same lot update, outside the repository, with a script
  that builds our state from the install and the house file alone. One
  line per thing compared. Equal, after each tick: the tick count; the
  lot's and the game's random streams; the scheduler cursor; the funds;
  all 64 globals; the removal queue; the registration order; the set of
  objects; per object its GUID, data words, temps, attributes, scheduler
  word, last result, frames with their saved primitive state,
  relationships, slot count, position and two marks; per person the 256
  person data words, motives and previous motives bit for bit, the
  running and last actions, the queue and the objects in use; the
  people's animators, in every field we keep that the capture's memory
  pages hold (100 readings after ticks 3 to 5, 72 after tick 2; 23 fall
  on pages the capture lacks); and, read from the neighborhood file the
  capture's game read, the 463 neighborhood records, the inventories and
  registry byte 0x12.
- The tick-1 comparison is unchanged: equal except the C runtime stream
  and registry byte 0x12, which our tick 2 sets and the capture already
  had set after tick 1.
- The comparison of the load with its trace still gives the three known
  differences, all in object 19's frames.
- Synthetic tests cover the spiral's order, wall crossing, the search
  staying in a room, flag 2, a refusal, the native errors, and mode 8's
  success, refusal and loud failures. The gated test runs ticks 1 to 5
  on the Goth house, pins the state after tick 5 and asserts the tick 6
  stop.

What did not match, and why:

- The C runtime stream is 52 draws further on in the capture after every
  tick. Neither side draws from it in ticks 1 to 5: ours stays at the
  same value and so does the capture's. The likely cause is that the
  capture's game drew 52 more numbers before its load; that is
  **unverified**.
- Read from the install's neighborhood file, records 47, 48, 301, 344
  and 351 and record 301's inventory differ after every tick, because
  the install's file differs from the one the capture's game read.
- Registry byte 0x12 differs after tick 1 only (ours 0, the capture 1).
  What sets it before tick 2 in the original is not known.

What is **unverified**:

- Global 0x6e4ad4, which would make the search refuse more tiles, is
  taken to be 0; the capture does not hold it.
- Everything after tick 5, including Create New Object Instance mode 8.
- The paths of Find Location For and the place test that the Goth house
  does not reach. They fail loudly where they are not ported; the
  ported ones beyond what ticks 2 to 5 use are checked only by the
  synthetic tests against our reading of the code.

Tick 6 starts at tick count 88,449. Object 382 asks for a new object of
type 0x44A86A6C, a person, where the pedestrian portal (object 6)
stands. Making and placing a new person is not ported, so the tick
fails there, before anything is made. What would reopen this: a capture
past tick 5, or a difference found in a later tick.
