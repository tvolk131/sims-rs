# A headline icon keeps its priority and duration; the social ends

*Decided 2026-11-15.*

**Unverified.** No captured update shows a headline: the Goth captures
stop before the social. The port is read from the original's code.

Set Balloon/Headline (0x44b724) now shows an icon, kind 6 among them.
It touches only the object's first headline record (object +0x148,
0x3c bytes each), so only one per object matters. The priority is 2 for
kind 4 and 1 otherwise. A record still showing with a higher priority
is left alone; otherwise its priority (+0x34) is written first, even
when the call then ends without showing anything. A duration of 0
clears it (0x46e042). Otherwise, from 0x44b7ed:

- The index is byte 2, plus my temp 0 (object +0x3a) with option bit
  12. Kind 7 is a catalog picture (0x44b811) and still fails loudly.
- The sprite is the kind's first sprite plus the index (the table at
  0x6c1f98, 0x416a0c): kind 6 is 600 and up. Kind 0 has its own table
  (0x666758), and kinds above 10 have none. No sprite ends the call.
- The frame count is the sprite's, from the headline sprite cache
  ([0x6f0c08], made at 0x4144c4 from `GameData/Sprites.iff`), divided
  by three (0x46dfb5). If that is 0 the call ends.
- The duration is kept as given, -1 included. With option bit 11 it is
  multiplied by the frame count and by 12 (0x46e050).
- The object is redrawn (0x434101, presentation) and woken (0x44574b).

The sprite cache (0x4c8e0d) looks up an SPR# and then an SPR2 of the
ID as resources (live chunks of the default variant, through the
resource map when there is one), not the file's raw chunks. The count it
hands on (0x4c8deb) is 0 below version 500, halved for versions 501 and
503, and divided by five for 506 (0x4c154e). `sims_vm::icon_frames`
reads every count once at boot into `Program::icon_frames`. An SPR#
header is swapped by the resource layer's callback (0x4c8f63) unless the
resource's conversion flag 0x10 is set (0x41d3a7): big-endian without
it, little-endian with it. We read the flag from the chunk header;
whether a resource map's entry can carry different flags is
**unverified**. All 195 SPR# in Sprites.iff have flags 0 and read
big-endian.

What the simulation reads of the record is kept in `World::headlines`
(appended; snapshot format 24): priority and duration, while the
duration is not 0. Its readers are:

- Sleep's passive test (0x44aa57), which needs no showing headline.
- The tail of an object's tick (0x4521f7 → 0x46e004), which counts a
  positive duration down.
- A person's reset (0x45e8af) and the destructor (0x4323ce), which
  clear it.

The sprite, the animation frame, the backdrop, the overlay and the
flag (+0x14 to +0x28, +0x2c, +0x30, +0x38) are only drawn and are left
out. That the drawn bounds (0x437ce8, 0x45a07b) feed nothing the
simulation reads is **unverified**.

Two small stops after it are ported too:

- **set_action with person word 73 set** (0x45e756): the word is
  cleared and the person's footprint worked out again, 0x433d16 with
  +0xf4 and +0x100. This is the same call Generic Sims Call routine 15
  makes (0x44fe9e), so it reuses that port. A facing the footprint does
  not handle fails before the action changes.
- **Go To Routing Slot, wanting to be interrupted** (0x45b18b, the
  test before the search for places): if the animator may stop
  (virtual function 0x30, 0x589ead), the route fails with result 4.
  The animator may stop in states 1 to 3; in states 0, 9 and 10 the
  foreground layer's +0x68 decides, or it may with none; in any other
  state it may not (`Animator::interruptible`). The same test at a
  door, on the way and while waiting (0x45afd2, 0x45b2c2, 0x45b480)
  still fails loudly.

Strict mode: through tick 1102 every line of `sims boot` matches main
except the final hash, now
3b539cc1f0cfddbfdc8e12a0e864e90528333317095306cf9463c9b7de5a6f1e. The
headlines stay empty in all those ticks. An empty map encodes as one 0
byte, and the encoding without that byte digests to the old
9b09ff4f78d81be329ef6d64a062bd6d1e4f72a530b977a3df83c57220cf5b18. The
tick-33 pin is re-pinned at format 24; its format 23 and 22 digests are
the old pins. The K=1000 tick pins in `resize.rs` hold unchanged.

**On Goth.** At tick 261 tree 0x200e shows "+ " (kind 6, index 16:
sprite 616, 9 frames, so 3) over person 21 for 100 ticks, and at tick
262 over person 16. The social ends at tick 264: word 73 is cleared and
the footprint worked out again. Both icons count down and stop showing
by tick 361. The lot then runs to tick 597, where person 259 (GUID
0x44A8C46C) fails its route with result 4 because it
wants to be interrupted. Tree 0x0176 node 27 then takes it off the lot,
which is not ported. That is where the next port starts.

The old prototype's `headline.rs` agrees on the kept fields and the
countdown. Reopen if a capture shows a headline's duration counting
differently, a drawn bound reaching the simulation, or a resource
map whose SPR# entry has flags other than its chunk's.
