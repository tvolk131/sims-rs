# Play mode resets an object after its error; strict mode is unchanged

*Decided 2026-11-15.*

For a playable lot, `Lot::step_play` (and `sims boot --play`) catches a
fault in an object's tick and goes on. `Lot::step`, `sims boot` and every
comparison stay in strict mode, which this does not touch: without an
error, a play-mode update is the strict one, and so is the lot it leaves.

What the original does on a SimAntics error is ported (`crates/sims-vm/
src/play.rs`): the thread's catch (0x474ec1) stores the code and sets
flag 4 through the object's virtual function 4 (0x439a4e), and the rest
of the tick runs. The scheduler then reads the error word, calls 0x439a6f
and stops the pass partial with the cursor on the object. With the debug
byte 0x6c2983 clear, 0x439a6f tells the other objects (0x444306) and
resets the object (its virtual function 0x38 with 0); the restart at Main
(0x47480c) clears the error word, so the next pass ticks the object as
usual.

These are **approximations**, play mode only:

- An unported primitive or service is handled as a SimAntics error.
- A fault in the rest of a tick (a person's animator, motives or room) is
  handled the same way, but the rest of that tick does not run. When it
  follows a fault caught in the thread, both are reported and the object
  is dealt with once.
- Where the reset fails (much of a person's is not ported), the thread
  restarts at Main and the animator resets where it can. Where that fails
  too, the object is parked: its scheduler flag 1 is cleared.
- An object that faults again on its first tick after a reset is parked,
  since the original would stop every pass at it and the clock with it.
  An object deleted in a play-mode update after its reset no longer
  counts, so a later object given its ID is not parked for its first
  fault. A deletion made outside a play-mode update is not tracked.
- A frame of the house view that fails is skipped; every failed frame of
  a step is reported.

Faults elsewhere in the update (dialogs, removals, the clock) still halt
the lot. On the Goth house with the K = 1000 work on top, 20,000 play
ticks run through; 47 errors are caught, and four people end up parked by
animator states that are not ported yet. Reopen any approximation when
the code under it is ported; they should then stop being reached.
