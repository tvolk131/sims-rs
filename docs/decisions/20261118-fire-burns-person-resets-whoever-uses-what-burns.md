# A fire that burns a person resets whoever uses what burns

*Decided 2026-11-18.*

Four pieces of the original are ported, all from its code. They were
found carrying the play-mode snack order from step 15925 to step 16218:
before Snap's slot offset was fixed (`20261115-snap-offset-order.md`)
the stove lit a fire at step 13199, and these were what the fire needed
next. With that fix the snack order lights no fire, so no boot test
reaches them now. Unit tests cover each piece. None is **verified**: no
recording has a fire.

**Removal at once through the lot's placement.** An object of one tile
that is not a person, removed at once (0x44123f) or reset as it is
deleted with no Reset tree (0x432055), leaves the lot by its virtual
function 0x50 with 1. For an ordinary object that is 0x433125, the
lot's placement (`Interpreter::take_off`): it is asked function 10, it
and what it holds move off the lot, and whatever held it is asked
function 9. That is the way now taken once the load has worked the
lot's rooms out. Before that, the placement cannot find the off-lot
place's room, so the narrow way the load was compared through
(`Machine::leave_lot`) is still taken. On the old run, step 15925
removed a counter under a fire this way, and the sink it held went too.

**Burn moves a person to its tile's centre** (0x44df25). The person's
position and level are read. It resets (virtual function 0x38 with 0)
and is put at the centre of the tile it stood on, at that level, with
the lot told (virtual function 0x58, called with the same arguments as
the fire's put). Then its virtual function 0x74 runs. The put is not
place-tested first, as the original does not test it.

The reset can restart the very thread running the Burn: the person may
burn itself, or its reset may reset the object that runs the Burn.
What the original does next on the restarted thread was not seen, so it
fails loudly. The check is general: if
any primitive that branches leaves the running object's stack at a
different depth, tree or node than before it ran, `Machine::step` fails
as unsupported rather than branching on the new thread. The work the
primitive did is kept, as with any fault partway through one. The stack
object is not part of the check, because an expression can set it.

**A reset of a person already in the middle of one is skipped** (object
flag 0x800, 0x45e7b2), where it used to fail loudly. On the old run,
Mortimer's last use record named himself; when his reset ended that
use, the object it reset was Mortimer, who skipped it.

**The deletion mark is kept while a deletion runs.** The original sets
object flag 0x40 before people let go of the object (0x441039, and
0x440fbc for every tile of a group). The object keeps it until it is
freed. A reset of the object meanwhile reads it: an object with no Reset
tree is taken off the lot rather than rebuilt, and a person is not moved
back onto the lot. That includes the reset a person makes as its reset
ends its use of the object, and the full reset of a person told to let
go of something while its own deletion runs. The interpreter now
carries the marks for the length of a deletion (`Interpreter::deleting`).

The full reset of a person letting go of an object it uses (0x45e814)
came in with Snap's slot offset (#244). On top of it, a failed reset
that had released uses now takes back the sound events, callbacks and
deletions it noted, not only the lot. Its check that an object it used
went as its frames were popped (0x45e86e) now applies only when that
object is to reset: with flag 0x1000 the original does not touch it.

None of this changes the strict Goth day: its hashes are those of
`20261115-replay-draws-recording-frames-queue-listing.md`.

**Where the old run stopped.** At step 16218 fire 46 burned person 27,
and person 27's reset made Mortimer and person 12 let go of fire 41,
which all three used. Person 12 sat on a sofa. For a person inside an
object, the original takes it off the lot and tests and searches from
where it is then. It discards what it finds, takes the person off again,
and puts it at or near the lot's centre (0x45e8dd, 0x45e977). Taking a
person off the lot refreshes its animator only if the house view drew
it, which is not kept. A reset person stays on the lot afterwards, so
that refresh would have to be held until its animator's next tick,
possibly in a later update. That is not ported, and it fails loudly.
