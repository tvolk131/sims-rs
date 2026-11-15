# Snap to a routing slot, and K = 608

*Decided 2026-11-14.*

Tick 584 stopped at Snap (opcode 46). Person 18 snaps to routing slot
0 of object 207. The slot has no offset and flag 0x800. The capture
shows the person afterwards where 207 stands, on level 2, in room 1025.

**Decided.** Port only mode 3 (a routing slot of the stack object).
Within it, port only the case the house reaches: no snap target, flag
0x800, operand flag 0 clear, and a snapping object that fits. That
object is a person in the house; an object that is not one snaps the
same way, without the animator. Everything else
fails loudly with its address. Each of the other branches needs code
we do not have:

- the placement's own checks without flag 0x800 (`0x4550d8`);
- the container slot for a snap target (`0x454b19`);
- the animator offset that operand flag 0 keeps (+0x8c, read when the
  animator places itself);
- the error words the failure path writes (`0x454e09`), including a
  person-to-person test (`0x45bd5e`).

**The float round trip, kept for a zero offset only.** The original
turns the slot offset by the object's facing through a sine and a
cosine (`0x455070`, `0x5c39a0`). With a zero offset the matrix adds
nothing, and the position goes through ×3/16 and back ×16/3 with
round-half-away. A coordinate times 3/16 is exact in a float. The
error of 16/3 as a float moves the result by less than 0.001 for any
coordinate up to 0x8000, so the round trip keeps the position. A
non-zero offset (the third coordinate is never read), or a position
further out than that, fails loudly.

**Order of writes.** The order follows the original:

1. the place test, then "set object" with the lot told (0x433340);
2. the facing;
3. the animator unset (0x589361);
4. data word 52 (+0xb2) cleared;
5. the animator's offset zeroed and the animator placed again (its
   virtual functions 0x2c and 0x60).

The animator's +0xd9 byte and the offset fields stay unmodeled, as
before.

**The upper level's floor test** (`0x4330b7`) is ported as one more
refusal. It has two codes, 0x12 and 0x29, which differ only by the
level the player is looking at; no caller tells them apart.

**Result.** The boot equals the capture through tick 608. Tick 609
stops in removing object 260.

**Reopen** when a later tick reaches a branch that fails loudly.
