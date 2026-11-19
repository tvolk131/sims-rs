# The cursor's hover runs check trees on the selected Sim, and is not ported

*Decided 2026-11-18.*

`20261118-set-to-next-follows-group-links.md` left one difference open:
at update 3350 of track 2's recording `goth-day-20261115`, person 21's
last result (object +0x20) is 1 in the original and 0 in the port. They
match again at 3360. Person 21 is the selected Sim (flag 0x2 at +0x114).
The original also has +0x1c, the last branch target, at 0xfe (return
true). Its frames are the same as the port's.

**Where the write comes from.** The interpreter writes +0x1c and +0x20
when a tree branches or returns (0x474cea to 0x474d27, 0x474d94 to
0x474da6). A tree that runs to its end on the person's stack, and is gone
before the update ends, leaves exactly this: the frames as they were,
and the last result and target of its return. In live mode the
interface runs such trees when the cursor moves to another object:

1. The mouse handler 0x48c924 reads the event's position, picks the
   object under the cursor, and calls 0x48c834 when that object
   changes.
2. 0x48c834 takes the highlight off the old object and calls 0x48d392.
3. 0x48d392 gets the selected Sim (0x4456e4) and builds the hovered
   object's interaction list for it: 0x458583, then 0x458319 for each
   entry, then 0x458505, which runs the entry's check tree with the
   synchronous run 0x4749b5 on the selected Sim's stack.

In the recording the cursor stays parked at about (492, 378) after the
clicks of updates 1 to 3 (`input-log.txt`). An object that moves under
it, or a camera change, changes the hovered object without any input.
Which object was under the cursor at 3350 is **unverified**: the
recording keeps neither the picks nor the frames' contents.

**Decision.** This is the interface's work, driven by the cursor, the
camera and the drawn frame. The port keeps it out, as it keeps out the
other presentation. The difference at 3350 is taken to be this hover,
and is **unverified** in that the hover run was not caught in a capture.

**What could make it matter.** A check tree the hover runs can write
anything a check tree can:

- the selected Sim's last result and target, which a later primitive
  could read before the next tree return overwrites them;
- the shared advertisement record (`World::advertisement_record`), which
  0x458319 copies each entry over;
- a random stream, if the check draws.

Here none of that lasted: every object's state matches again at 3360,
and the lot and global streams match through update 4154. Wherever the
recording leaves the port again with no other cause, the cursor's hover
is a candidate. At 4155 the recording's global stream moves for the
first time since the load, and that is not yet worked out.
