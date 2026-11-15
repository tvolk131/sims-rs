# Objects are visited up to the first person, and frames keep what earlier frames left

*Decided 2026-11-13.*

After the household is selected the original visits every object on the
lot, in registration order (load step 13). `sims boot` now makes those
visits for objects that are not people, and stops at the first person. On
the Goth house that is 201 of 382 visits.

Some terms. An object's *callback table* lists the trees the game runs on
set occasions; "function 2" is the second entry counting from 0. Function
0 is Init, 2 is Load, 3 is Reset, 9 is run when the object has been placed
and 10 before it leaves. A type has *changed* when the scripts installed
now differ from the ones the house was saved with (step 7). To *rebuild*
an object is to take it off the lot, reset its storage, run its Init and
give it a first turn, and put it back.

What was decided:

- **A visit is `Interpreter::visit`** (0x431e2e). The object's position is
  written again, with its footprint and room, for it and everything it
  holds. Its Load function runs. If its type changed, it resets, and then
  Init runs. Two scheduler flags are refreshed. A reset runs the Reset
  function if the object has one, and otherwise rebuilds the object.
- **The step stops at a visit it cannot make.**
  `Interpreter::visit_objects` makes each visit on a copy of the lot and
  keeps the copy only if the visit finished. So the lot is always in a
  state the original is in between two visits, and the result says which
  visit failed and why. A person's visit fails: it needs the person's
  fields from the neighborhood record, the person's own reset, its room
  and its motive curves, none of which is ported.
- **Placement is `placement.rs`.** Taking an object off the lot and
  putting it on a tile or into another object keeps three things in step:
  the object grid, the slots, and each object's link words (data words 14
  and 26, and 2 and 3). The callbacks run in the original's order. A
  multi-tile object moves as a group.
- **The frame storage is state** (`World::frame_storage`). The original
  keeps a thread's frames in one block of memory, one after another, and
  does not clear the block. A new frame's locals are not zeroed: they hold
  what an earlier frame left at that place. We now keep that block for
  every object whose thread has pushed or popped a frame, write the live
  frames into it at each push and pop, and take a new frame's locals from
  it. Before the original runs a callback it writes an eight-word mark
  above the top frame; we write it too, because it moves where the
  callback's frames lie. Two words of each frame's header hold a pointer.
  A local that lands on one cannot be read until it is written; reading it
  fails and says so.
- **The lot's indicators are made in step 13** when the lot has none and
  an object comes onto it from outside (0x44752e calls 0x445e4a). In a
  load that is the first rebuilt object that stands on the lot.
- **An object with graphics of its own draws 32 numbers** from the C
  runtime's random stream when the factory makes it. The indicators are
  such objects. Only the draws are kept. This also settles the stream's
  state at the start of a load, which earlier entries called state of the
  running game that no file holds. In the trace the stream starts the
  load at its initial state 1 advanced by exactly 64 draws: the two
  indicators the game made when it started. `sims boot` makes those two
  on its starting lot, so the stream now equals the trace's at every stop
  from the files alone.
- **New primitives**, each for the cases the visits reach: Notify Stack
  Object out of Idle (49), Relationship (26) and Old Relationship (24),
  Create New Object Instance (42), Run Tree By Name (28), Set To Next
  modes 8 and 9, Refresh modes 1 and 2, and Generic Sims Call 10, 11 and
  43. What each leaves out fails with an error that names it.
- **The snapshot format is 7.** `World` has two new last fields: the list
  of fires and the frame storage.

How it was checked. The private trace of the original loading the Goth
house holds a full state at four moments of this step: its entry; the
entry and the return of the function that creates the indicators, which
runs in the middle of visit 140; and the step's return. For every visit
it also holds each call of the function that runs an object's callback
(0x422d46), with the object and the function's number. Our run was
compared with it outside the repository
(`visits_verify.py` in the private scratchpad).

*Equal to the trace:*

- **Every call of every visit that runs.** For each of the 201 visits:
  the object visited, and the calls it makes, as object and function, in
  order. 525 calls.
- **The whole state in the middle of visit 140**, at the entry of the
  indicators' creation. All 382 objects: GUID, the 72 data words, temps,
  attributes, scheduler word, last result, every frame's tree, node,
  stack object, parameters, locals and saved primitive state, the
  relationships, the number of slots, the position, the
  removal-without-resets mark, and the three people's data. Also the
  tick, the 64 globals, both game random streams, the funds, the removal
  queue (seven controllers asked to be removed in their Load trees), the
  registration order, the instance lists, which rooms wait to be counted
  (none), and the arrays inside the lot's size, the object grids
  included. By then 139 visits are over, 137 that only run Load and two
  that rebuild a controller off the lot, and visit 140 has taken the
  first object off the lot, rebuilt it and put it back.
- **The same when the indicators' creation returns**, with the two new
  indicators, and the C runtime's random stream, which is 64 draws
  further on. Object 246 is left out there, because the trace stops in
  the middle of its visit, and the global random stream, because the
  rest of that visit draws from it.
- **The 205 objects visited or created by visit 201**, against the
  trace's state at the step's return: every object field listed above.
  17 of them differ from the trace in data words 14 and 26 and in nothing
  else. In the trace each of the 17 is linked behind an object that is
  visited after visit 201. That object's rebuild takes it off the tile
  and puts it back in front, which rewrites those two words of the object
  behind it. So they are what they should be after visit 201.
- **The frame storage's effect.** The locals of restarted Main frames
  hold in the trace what our frame storage gives them: for example object
  12's one local is 280, the tree ID of the frame that used to lie where
  the local now does.

The comparison is not empty: between the step's entry and visit 140 the
trace changes 10 scheduler words, 10 attribute sets, the frames of 9
objects, global 36, the global random stream, the removal queue and an
instance list, and all of those are equal here.

*Consistent with the old prototype's isolated comparisons only.* These
run on the Goth house too, but the trace cannot show them wrong or right
in detail, or they do not happen there:

- Placement beyond what the objects rebuilt in visits 138 to 201 do:
  an object put into a container whose parent is asked function 9, the
  seat flag, the door mark on a tile, the wall bits of data word 5 (four
  objects are asked function 6 in the trace, and here), and a group
  placed off the lot.
- The footprint. The trace's objects hold it, but it was not compared.
- Set To Next modes 8 and 9, and the Relationship primitives beyond the
  rows they write here.
- That a tile inside the border of the 64 by 64 arrays counts as on the
  lot. The prototype tests against the arrays, not the lot's size, and no
  object of this house stands between the two.

What did not match. Object 19's second frame has tree ID 0x8118 in the
trace and 0x118 here and in the house file, as before. Its visit comes
after visit 201. In the trace that frame is gone when the step returns:
the object's reset replaces its thread. Nothing here explains the bit.
Six type attribute words belong to the game that was running; with those
copied from the trace's start they are equal. The C runtime's random
stream is now equal without copying anything.

What changed in earlier work, and why:

- **The pinned digests of steps 3 to 12 changed** because of the two new
  fields and the indicators' 64 draws. The gated tests show each state is
  the one pinned under format 6: with the fires and the frame storage
  emptied and the stream put back to its old state, the encoding hashes
  to the old digest. The comparison of steps 10 to 12 with the trace was
  run again and is equal as before.
- **The people's constructors draw 7,084 numbers, not 7,074.** They draw
  until a value fits, and they now start 64 draws later in the stream.
  Nothing they draw is kept. The stream's state after them is the
  trace's. Before, it was only once the stream's state at the start of
  the load had been copied from the trace.
- **An earlier note said function 2 is the only thing asked of 191
  objects.** It is 198: seven more run a Load tree that only wakes the
  object.
- **Refresh modes 1 and 2 no longer fail.** They mark the object's room
  as not counted.

What is **unverified**:

- **Everything after visit 140 that is not an object field**: the global
  random stream, the globals, the arrays and object grids, the removal
  queue and the rooms waiting to be counted after visits 141 to 201. The
  trace has no state there. The object fields at the end of those visits
  are equal, which the random stream and the grids feed into.
- **The contents of the slots and of the frame storage.** The trace holds
  neither. The link words and the object grids, which mirror the slots,
  are compared, and so are the locals that come out of the frame
  storage.
- **That frame storage never written reads as zero**: beyond the frames
  a save holds, and in a new object.
- **A person that refers to an object that resets.** The original makes
  the person drop it. Here that fails. No person of this house refers to
  an object reset in visits 1 to 201.
- **Notify on an object that is inside a callback further up the chain
  of calls.** We look at its top frame.
- **Create New Object Instance for a GUID with no definition**, which
  takes the false branch here.
- **The lot before a load**, as before, but for one thing the trace now
  confirms: its two indicators' 64 draws.

What is not modeled: the redrawing, shadows and lighting an object asks
for as it moves; the interface being told an object left; mode 1 of
Refresh asking for a redraw; and the old-version conversions the original
applies to an object between restoring its position and running Load.
