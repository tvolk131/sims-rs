# The end of a path on a timer, the walk's strides, and K = 333

*Decided 2026-11-14.*

In tick 262 of the Goth house, person 21 comes to the end of a path
with object data word 9 clear. That case is now ported, with what it
needs: the walk's strides, worked out by posing the person's skeleton.
Ticks 262 to 333 then reach a few more mechanics, which are ported too.
Our state after every one of ticks 1 to 333 equals the fourth capture.
Tick 334 stops loudly. K goes from 261 to 333.

Terms used here. A *stride* is how far apart a person's feet are, along
one axis, at a footfall of the walk clip; the animator keeps two
(+0x17c and +0x178). A *footfall* is a phase of the walk clip marked by
an `anchor` event. A *pose* is where every bone of a skeleton is when a
clip is played at a phase. A *sample* is a bone's position or rotation
at one frame of a clip, stored compressed in a CFP file.

What was decided:

- **The end of a path with data word 9 clear is ported** (0x58e8be to
  0x58eb03). The phase of the walk clip picks the next footfall and the
  stride that ends there. Within that stride of the end, the animator
  sets a timer (+0x1c0) and fades the clip's weight to the share of the
  stride left (0x597a3e). A clip that may not be interrupted and has not
  finished only sets the near-end flag. When the timer runs out, the
  person stands on the end point, facing along the path's last segment.
- **A layer's fade is ported** (0x597e3c to 0x597e9b). The old test that
  expected a fading layer to fail now checks the fade instead.
- **The strides are worked out when needed, not kept.** The original
  works them out once, in the person's constructor (0x58831d), and keeps
  them. We work them out from the same walk clip and skeleton each time
  (`Interpreter::strides`). The result is the same while the clip and
  skeleton stay the same, and the snapshot does not grow. The pose code
  is new (`sims_sim::pose`). It follows the old prototype's 24-bit
  variants (`Selection::at_position` and `motion_with_control` in
  legacy/crates/sims-sim/src/pose/sampling.rs;
  `blend_quaternion_with_control` and `compose_bone_with_control` in
  legacy/crates/sims-sim/src/pose.rs). Content now reads each
  skeleton's bones and each clip's channels and raw samples.
- **The sample decoder uses the original's table.** The original turns
  each compressed sample byte into a change through a table of 253
  floats that it builds at 0x5975d4 (stored at 0x6f7188). Each entry is
  worked out in steps, each rounded to single precision. Our decoder
  used a closed formula in double precision, which differed in the last
  bits of most entries. That moved the strides by 70 to 400 units in the
  last place. `sample_deltas` now builds the table as the original does.
- **The turn on the spot takes every turn clip** (0x58f4ee to 0x58f688),
  not only the half turn. A clip already in the foreground is cleared
  first (0x592878).
- **Animate Sim names a clip from an object's own tables** (source 0,
  0x59dd6d): string tables 129 to 132, by the ages of the person and of
  the person its interaction is with.
- **Look Towards mode 0 is ported** (primitive 22, 0x45a98e, 0x58a035):
  person data words 41 to 45 hold the head's target. Modes 1 to 3 fail
  loudly.
- **Snapshot format 17** keeps the motion's timer (+0x1c0) and each
  layer's fade (+0x48 to +0x54). The pins taken under format 16 still
  hold with these left out (`format_16_bytes`).
- **The harness reads a stale spatial-order cache without failing.** The
  fourth capture stopped at update 609 with `unknown retained
  spatial-order object identity`. The registry keeps a cache of objects
  in drawing order (registry +0x4c). The only reader (0x440349) rebuilds
  it first (0x440397) when its dirty byte (+0x44) is set or its rotation
  is not the camera's. Removing an object (0x44383c) sets the dirty byte
  (0x443b65) and leaves the cache as it was. So a dirty cache can hold
  the address of an object that is gone. The harness now keeps a stale
  cache's addresses as read, with no identities, and still fails on an
  unknown address in a current cache. Its output schema is now 2.

How it was checked:

- After every one of ticks 1 to 333, our state equals the fourth
  capture's, floats bit for bit, in the same fields as before, now with
  the timer and the fade, and the `updates.jsonl` line.
- The second capture is equal at every tick it holds (to 240). The third
  is equal to 300 but for the two known lines (global 12 after ticks 1
  and 2; one local of object 18 after ticks 210 and 220).
- Our strides equal the stored ones bit for bit, for all three people
  (child 16: 0x40d893f8 and 0x40bcad30; adults 18 and 21: 0x412c5b86
  and 0x412a0544). The capture holds the same values after updates 1,
  300 and 600.
- Our table of 253 changes equals the one in a memory image of the
  original at 0x6f7188, entry for entry. In double precision, 225
  entries differ.
- `the_goth_house_first_tick_matches_the_original` runs to tick 333,
  checks the end of the paths in ticks 262 to 322 and the strides, pins
  the state, and checks that tick 334 stops at 0x5904d2.
- The load comparison still gives its three known lines.
- New synthetic tests: the frame choice, sampling, blending and
  composing of a pose; the fade; the start of a stop on a timer; the
  table's digest; Animate Sim's own tables; and Look Towards mode 0.
- The harness change was checked only on made-up memory. No capture
  has been run with it.

What did not match, and why:

- Nothing new. At the stop in tick 334, person 16's person word 27 is
  12 in ours and 13 in the capture after the tick. The steps after the
  stop presumably write it, as at the stop in tick 262.
- The first capture still differs from tick 233 on, as before.

Unverified:

- To pose the skeleton for the strides, the layer's time since its last
  pose is taken as 0 (so the next frame is always blended in) and the
  clip as looping. Neither is read from the original. The strides this
  gives equal the stored ones.
- That a removed object was what stopped the fourth capture at update
  609. The code says a dirty cache can hold one, and the dirty byte was
  set after every update the capture saved; the memory at update 609
  was not saved.
