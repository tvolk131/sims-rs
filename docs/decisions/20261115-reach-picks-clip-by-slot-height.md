# Reach plays a reach clip picked by the slot's height and moves the object at event 0

*Decided 2026-11-15.*

Reach (primitive 47) was the most used primitive the port did not have.
The base game's trees use it 16 times: getting food, picking plates up,
the food slot test and food placement, paying bills, throwing flowers
away, the bar and the thief. A day with meals reaches it. It is now
ported (`primitives/reach.rs`) from the person's own dispatcher
(0x45cbb6), which sends opcode 47 to 0x45c9ba.

- **Not a person.** The general table (0x45246a) has no entry for 47.
  Its default throws native error 5 (0x452455), and so does the port.
- **The steps.** The frame's primitive state holds the step:
  - 0: start the reach clip, or branch at once.
  - 1: wait for event 0 or the clip's end, then move the object.
  - 2 and 3: wait for the clip's end, then end the reach with the true
    or the false branch.
  The module doc lists each write. The ending (0x58a00d) is not Animate
  Sim's (0x589c25): it does not touch whether a clip plays (+0xa0), the
  base layer or person word 19.
- **What it reaches for** (0x45c848):
  - Mode 0: the stack object, in the container and slot it is in.
  - Mode 1: the stack object's slot that a frame parameter names, and
    what my hand holds.
  - Mode 2: nothing, at height -1.
- **The height.** The heights of the slots up the chain of containers
  are added (0x43404d). A container slot's height (record +8) is what
  its SLOT height code makes it (0x46def8):
  - code 1: 0;
  - code 2: 2.5;
  - codes 3 and 8: 4;
  - code 4: 4.05;
  - any other code: the entry's third offset.
  A person's own five records are at 0. The sum is scaled by 1/(2√6)
  (0x454f29) and by 3, each step stored as a float.
- **The clip** (0x59265c):
  - Below 0 is clip 9, below 0.75 clip 1, below 2 clip 3, below 2⅔ clip
    5, below 4 clip 7. A sitting person takes the next number up.
  - The names come from the global file's string table 154 (155 for a
    child), which lists them as floor, seat and table reaches with
    sitting variants.
  - At 4 or above no clip starts. The branch is then true when nothing
    is to move and false otherwise.
  - A number the table does not list, or lists as an empty name, is no
    clip either (0x59dd09 hands on a null name, 0x589f45 fails the
    start), with the same branch.
- **The clip's start** (0x592726) is shared with Animate Sim
  (`start_foreground`), which now calls it instead of its own copy. The
  steps are the same.
- **The move.** Mode 0 runs Grab's place test and put, into my slot 0.
  Mode 1 does the same for what I hold, into the named slot. Then the
  moved object's "CT - Grab" or "CT - Put" tree runs at once on my stack
  (0x452bed), with my temps as its arguments.

**Fails loudly:** a listed reach clip that our content lacks (the
original reports a missing animation and plays a stand-in), and a put
that fails on what the port does not model.

**Unverified.** No capture covers a Reach yet. The tests are synthetic.
A run that reaches it (a meal in a day recording) should check the clip
choice and the step at which the object moves.

**Strict runs.** The Goth and Newbie strict runs are unchanged. Neither
reaches a Reach, or it would have stopped on the unimplemented
primitive.
