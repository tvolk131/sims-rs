# The difference at update 3350 is the pointer's pie menu listing, which the strict replay has no input for

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
interface runs such trees when the pointer reaches another object:

1. The mouse handler 0x48c924 reads the event's position, picks the
   object under the pointer, and calls 0x48c834 when that object
   changes.
2. 0x48c834 takes the highlight off the old object and calls 0x48d392.
3. 0x48d392 gets the selected Sim (0x4456e4) and lists the object's pie
   menu for it: 0x458583, then 0x458705, which calls the check 0x458319
   for each entry, and 0x458505, which runs the entry's check tree with
   the synchronous run 0x4749b5 on the selected Sim's stack.

The port has this listing: `Lot::pie_menu` (0x48d392), which the game
runs when the pointer reaches an object, including an object that moves
under a pointer standing still (see
`20261115-game-lists-pie-menu-lot-when-pointer-reaches.md`). What the
check trees change stays on the lot, as in the original.

**Why the strict replay differs.** The strict replay of a recording
(`Lot::step`, as the whole-day test runs it) has no pointer. In the
recording the pointer stays at about (492, 378) after the clicks of
updates 1 to 3 (`input-log.txt`), so an object moving under it, or a
change of view, lists a pie menu without any input. The recording keeps
neither what was under the pointer nor the frames drawn. Which object
was listed at 3350 is **unverified**, and so is the claim that a listing
made the write: it was not caught in a capture.

**Decision.** The difference at 3350 is taken to be such a listing, and
the strict replay is left without it. Driving the listing in the replay
would need the recording's pointer and what the original drew under it.

**What it could change.** A listing runs check trees on the lot, so it
can write what any check tree writes:

- the selected Sim's last result and target, which a later primitive
  could read before the next tree return overwrites them;
- anything else a check tree writes, such as globals or object data;
- a random stream, if a check draws.

The shared advertisement record (`World::advertisement_record`) is not
touched by the listing itself: 0x458705 passes 0 as the check's second
argument (0x458711), so 0x458319 skips the copy at 0x4584af. A check tree
can still write the record.

Here none of that lasted: the fields `cmp_day` compares (data, temps,
attributes, last result, frames with their locals and arguments, person
data) match again at 3360, and the three random streams match through
update 4154. Wherever the recording leaves the replay again with no
other cause, a listing under the parked pointer is a candidate.
