# People let go of an object that resets or is deleted

*Decided 2026-11-15.*

Before an object resets, and when it is deleted, each person on the lot's
list of people, in its order, lets go of it (0x444306 calling the
person's reset 0x45e791 with the object; `crate::forget`). Objects that
are not people are not told, and a person object not on the list is not
told either. A person that does not use the object filters it out: the
queued actions naming it are dropped, each one's target told it was
cancelled (function 4, so an action on the object tells the object
itself) and the rest kept in order; the last action naming it becomes
the empty action, or loses the icon; the running action naming it ends.
When the object is a person, route contexts asking it to move forget it.
A person nested in a broadcast it set off is skipped, as the original's
object flag 0x800 skips it; that flag is carried by the interpreter for
the broadcast's length, not kept in the world. A person's own reset sets
the same flag for its length, so its callbacks' broadcasts skip it too,
and a reset of a person already flagged is skipped, as in the original
(at first it failed loudly; see
`20261118-fire-burns-person-resets-whoever-uses-what-burns.md`). A
broadcast while the factory makes a person fails loudly, as any walk
of the list of people does then. A cancel's fault keeps the tree and
node it happened at, and a failure partway leaves the queue no longer
than its ring of 8.

A person using the object, or whose running action shows its icon, is
reset fully by the original: its uses unwound and each used object reset,
then its own reset, and then the filter above. That was not ported at
first; it is now (`20261115-snap-offset-order.md`, and
`20261118-fire-burns-person-resets-whoever-uses-what-burns.md` for the
deletion mark and the rollback).

What the filter leaves naming the object, the original leaves as a
pointer to freed memory: the last action after the running action ends,
a route context aimed at an object that is not a person, frames whose
stack object it was. Here they keep its ID, which a later object may be
given. A deletion with a route aimed at the object still fails loudly
before anything changes. Reopen when a capture shows a deletion with
people on the lot; none of this is **verified**. Ported from the
original's code; the old prototype's `person_reset.rs` reads it the same
way. Play mode reached it at tick 1023 after a player's snack order.
