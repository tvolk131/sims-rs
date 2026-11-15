# The head follows its target, and K = 366

*Decided 2026-11-14.*

In tick 334 of the Goth house, person 16's head starts to follow what
it looks at. That is now ported, with what it needs: the places of the
head's points in the current pose, and the height of the ground. Ticks
335 to 366 then reach a few more mechanics, which are ported too. Our
state after every one of ticks 1 to 366 equals the fourth capture, but
for the last place of one head field. Tick 367 stops loudly. K goes
from 333 to 366.

Terms used here. A *heading* is an angle seen from above, in eighths of
a turn from 0 to 8. An *anchor* is a named point fixed to a bone, such
as the top of the head. The *current pose* is where every bone is when
the foreground clip is played at its present phase. A *joining entry*
is the interaction a person may start to take part in another person's
running interaction.

What was decided:

- **The head's follow, relax and settle are ported** (0x590768,
  0x5907c5), in `sims_sim::head`. The turn moves by at most 0.2 eighths
  per tick towards the difference of the two headings, and stops at 1.5
  either way, where word 42 changes. With the follow off it shrinks by
  a fifth each tick. Look Towards (frame step 6) now feeds it.
- **The headings are measured on the current pose** (0x5904d2). The
  pose is built when needed from the foreground layer, as the strides
  are, not kept. The HEAD bone is found by name and turned for the
  measurement only, then turned back (0x590b27). The person's model
  matrix (0x592132) turns the skeleton to its direction and lifts it to
  the height of the ground. Every add, subtract, multiply, divide and
  square root is rounded to single precision, as the game's FPU setting
  does.
- **The height of the ground follows the tile's corners** (0x478dfe).
  The tile is cut along its diagonal into two triangles; the height is
  taken on a plane through the triangle's corners. The original hands
  the function the person's rounded second coordinate first, then the
  first (0x592234, 0x592242). The old prototype's `Terrain::height`
  (legacy/crates/sims-sim/src/pose/transform.rs) has the same shape.
- **Snapshot format 18** keeps the head's fields (animator +0x210 to
  +0x224). The target is None until something writes it: the original
  leaves those words as memory held them. The pins taken under format
  17 still hold with them left out (`format_17_bytes`).
- **A walk can start with a clip** (animator state 6, 0x58e125). The
  person moves one clip's distance straight ahead, following the clip
  until its phase reaches 0.95.
- **Find Best Interaction scores joining entries** (0x41ee99, called at
  0x458aee). For a person whose running interaction may be joined
  (flag 0x2), the entry is that interaction's joining index on the same
  object. Its check tree runs with arguments 1, 1, 0, 0 instead of 1, 0,
  0, 0 (0x458372). When the joining interaction is missing, the
  original leaves the entry's target empty and 0x458319 returns before
  any effect, so we leave the entry out.
- **Run Tree By Name mode 1 is ported** (0x44ba26, 0x44bb4b). The tree
  runs to its end on the stack object's thread, with that object's
  temps (0x4749b5 with no arguments). No one is woken and no use is
  counted. Mode 0 still fails loudly.
- **The place test's refusal is told apart** (0x4337f6, read by
  0x48158c). A wall crossing answers 3. An object in the way answers 6
  when it is the route's own first object, else 5 for a person and 1
  for any other, and its id goes to the route's +0x8c. An exclusive
  clash with no object answers 6. Refusals the original marks with
  other codes answer 0. A refusal whose code we have not read (the
  wall rules of step 3) fails loudly at 0x48158c.
- **The planner gives up with no candidates** (0x482462), setting its
  result to 7 when it was 1. The route to a slot then clears its
  candidates and both indices (0x481898) and answers 0. Go To Routing
  Slot's step 6, which takes that answer, fails loudly at 0x45b066.

How it was checked:

- After every one of ticks 1 to 366, our state equals the fourth
  capture's, floats bit for bit, in the same fields as before, now with
  the head's fields (`tick_verify_long.py`, outside the repository).
- The height of the ground: from tick 356 on, person 259 stands on
  sloped tiles. The heights the capture implies at eleven points fit
  only the corner order above with the coordinates handed over in that
  order; the disassembly at 0x592234 confirms it.
- The second and third captures still agree where they hold a state.
- `the_goth_house_first_tick_matches_the_original` runs to tick 366,
  checks person 18's walk start in tick 335 and person 16's head target
  in tick 357, pins the states after ticks 333, 334 and 366, and checks
  that tick 367 stops at 0x45b066.
- The load comparison still gives its three known lines.
- New synthetic tests: the follow, relax and turn limits; the headings;
  the ground's height on each half of a tile; the walk start and its
  end at phase 0.95; a joining entry's check arguments; the refusal
  answers; and the planner giving up.

What did not match, and why:

- Person 16's head target height (+0x220) is one unit in the last place
  lower in ours after ticks 335, 336, 338 to 340, 342, 343 and 353. The
  height of the ground was checked exact on those ticks, so the
  difference is likely in the pose of the top of the head. The captures
  do not hold the bones, so it cannot be traced further now. Nothing in
  the simulation reads this field.
- Person data word 37 still comes from a candidate word the original
  never writes; it is held as unknown and not compared, as before.

Unverified:

- The sine, cosine and arc tangent come from libm in double precision.
  The original's FSIN, FCOS and FPATAN may round differently at some
  inputs; no difference was seen.
- That Run Tree By Name mode 1 answers false for an object whose thread
  is in an error state (0x4749b5 reads +0x28). No object here is in
  one.
- Which code the place test's wall rules (step 3) leave for 0x48158c.
  They fail loudly when reached.
