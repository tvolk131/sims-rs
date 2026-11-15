# A route asks a person in the way to move, and K = 367

*Decided 2026-11-14.*

In tick 367 of the Goth house, person 18's route finds person 21 in its
way. The route now tries again (step 6), asks person 21 to move, and
plays the asking animation (step 14). Our state after tick 367 equals
the fourth capture. Tick 368 stops loudly on Change Suit/Accessory. K
goes from 366 to 367.

Terms used here. The *asking person* routes; the *asked person* stands
in the way. A *marker* is the small object made where the asked person
stands; the asked person is given an action to walk to one of the
marker's slots.

What was decided:

- **Route steps 6, 14 and 16 are ported** (0x45b066, 0x45b475,
  0x45aef7), with the retry count (+0x58) and noted place (+0x60,
  +0x64) read at the start of each run of the steps and written back
  at its end (0x45b9a0), not put back as they were. Step 16 fails
  loudly where it would read the route's slot flags on a later run,
  because the original reads them through a null slot record there
  (0x45ad54 passes 0 after the first run). *Superseded by "A blocked
  step, and K = 466": every run reads the context's copy.*
- **The person in the way and the marker are ported** (0x45ba28,
  0x45bf2e), in `primitives/in_way.rs`. The lot's own list of people is
  not modeled, so more than one candidate fails loudly (0x45bb04): the
  order would matter. The marker's move slot is kept in a new map,
  `Routing::slots`, which Go To Routing Slot reads before the type's
  slot table; deleting an object drops its records.
- **Queueing an action is ported for the plain case** (0x45d20b): the
  next action number, then a priority insert; a full queue (8) takes
  nothing. Running at once, putting at the front and pushing queued
  actions out fail loudly. Taking an action out of a queue (0x45d5f8)
  calls each removed action's object with function 4 and then checks
  that the queue was not changed by those calls; the person running the
  action fails loudly.
- **Snapshot format 19** keeps the context's action number (+0x90) and
  the slot records. The format 18 pins still hold with them left out
  (`format_18_bytes`).
- **Slot flag 0x2000 adds a random part** to each place's score: one
  draw from the active stream, modulo 20, less 10 (0x483b02). The
  capture's 72 draws in tick 367 match one draw per place in range.
- **The slot search scores each place once.** The loop at 0x483a5d
  takes the place at the standing index or at the index after the
  seats, whichever weight is larger, never both. Our second pass over
  refused places was wrong; it never showed before, as classification
  has no other effect. This is a correction to an earlier reading.
- **Look Towards modes 2 and 3 are ported** (0x45a9c1): a route from the
  person's place to itself and then to the target (mirrored through the
  person in mode 3). Mode 1 reads the camera and fails loudly.

Not yet reached by a capture comparison: step 14's waiting tree, the
third and fourth retries, step 16 and the end-of-route release. They
are ported from the disassembly. The capture shows the waiting tree at
update 409 and the route ending at 489, so they are checked as K passes
those ticks.

The Sol review raised two findings, both rejected:
- *Function 4 should run after the action leaves the queue.* The
  original runs it with the queue intact (0x45d697 to 0x45d70b; see
  `release_action`).
- *Format 19 should still decode format 18.* Snapshots refuse any
  other format by design (`FORMAT`), as for formats 15 to 18. Saves
  with migrations are a separate format.

What would reopen it: a capture where a slot search with flag 0x2000
draws a different number of times, or a house with two people in a
route's way.
