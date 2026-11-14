# Decisions

Choices made while working toward `docs/roadmap.md`, newest first. Each entry
says what was decided, why, and what would reopen it. Entries marked
**unverified** wait on the comparison harness.

## 2026-11-13: Objects are visited up to the first person, and frames keep what earlier frames left

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

## 2026-11-13: The lot's rooms are worked out from its walls, and the load runs through the household selection

The original works out which room each tile is in from the lot's walls.
It does that after it has read a house's objects, and then gives every
object its room (load step 10), empties the removal queue (step 11) and
selects the household (step 12). `sims boot` now does all of that and
stops before step 13.

What was decided:

- **The rooms are state of the lot** (`World::rooms`), worked out by
  `World::rebuild_rooms`, which is the original's 0x479d9a. A room is a
  set of tiles that connect without crossing a wall. The map holds one
  room ID for each tile of each level. A tile that a diagonal wall cuts in
  two has the mark -5 there, and a record that names its two triangles and
  the room of each. The ground level's rooms are numbered from 0 and the
  upper level's from 1024, in the order a column-by-column scan finds
  them.
- **A room's tile list keeps its repeats.** A room lists its tiles in the
  order it reached them, and lists a tile again each time a neighbour
  reaches it while it waits. The original counts a room's size from that
  list, so a room of 27 tiles can list 46 and count as 46. We keep the
  list as it is.
- **The same function rebuilds the tile flags**, as in the original: which
  tiles are outside every enclosure, and which can be built on. The
  original has a branch for a flood that stands on a tile with a diagonal
  wall. A flood can only get there by starting there, at the lot's corner,
  and we refuse a diagonal wall on the border. So the branch is not
  ported.
- **Whether a room is outdoors is worked out with the rooms.** The
  original works it out the first time something asks and keeps the
  answer. Nothing between the two moments changes what it is worked out
  from, so the result is the same.
- **Step 10 is `Interpreter::assign_rooms`.** The first pass writes each
  object's room (`World::room_at`, 0x433bc5) into its data word 29. The
  second runs each object's function 7 tree as a callback. Scripts can now
  read three things about a room (scope 23): whether it is outdoors, its
  level, and its area. Any other room field fails and names itself.
- **The room count is `World::update_rooms`**, the part of the original's
  room update (0x466c3c) that needs only walls and floors: each room's
  half tiles, covered half tiles, walls and covered walls, and from them
  global 27, the lot's indoor area. The original runs its update after
  step 10 and before step 11, and so does `sims boot`.
- **Steps 11 and 12 are called.** `World::select_family` now also takes
  the family's six magic words and sets `World::magic_history`, which
  nothing set before. The family's ID is the ID of its record in the
  neighborhood file, not the number inside the record.
- **Refresh modes 1 and 2 still fail loudly.** They mark a room as
  changed. No tree in steps 10 to 12 reaches them, so nothing shows what
  else the original does there. Step 13 reaches mode 2.
- **The snapshot format is 6.** `World` has a new last field, the rooms. A
  lot with no rooms encodes them as four zero bytes. The gated tests show
  that each state pinned under format 5 is the same state without those
  bytes.
- **A lot with no wall array gets a bare lot's cells outside its size.**
  See "What was wrong" below.

How it was checked. The old prototype kept, privately, a trace of the
original loading the Goth house. It stops at the entry and the return of
step 10 (0x442659), at the entry and the return of step 11 (0x428679),
and when step 12's funds setter (0x46d8dc) returns. At each stop it holds
the objects, the globals, the random streams, the arrays inside the lot's
size, the 672 bytes of each room, and some pages of the game's memory. Our
state at each of the five moments was compared with it outside the
repository.

*Equal to the trace, the rooms (entry of step 10):*

- The room IDs: 0 to 4 and 1024 to 1029.
- How many tiles each of the 11 rooms lists.
- The listed tiles, in order, of the 8 rooms whose tile memory the trace
  holds: 1, 2, 4, 1025, 1026, 1028 and 1029 at this stop, and 1027 at the
  end of the load.
- The room of every tile of the upper level, in the 32 columns of 64 the
  trace holds at this stop.
- The number of split tiles, 7.
- The tile flags of both levels, every cell. The rebuild changes none: the
  flags it works out are the flags the house saved.
- For an empty lot of 52 tiles, which the dump tool makes by blanking the
  walls and floors: two rooms that list 5,307 tiles each, as the two rooms
  the trace has before the rebuild do.

*Equal to the trace, but at a later stop.* The room of every tile of both
levels, all 64 columns, equals the trace's memory at the end of the load.
The trace does not hold the ground level's map at the entry of step 10.
Its rooms have the same tile lists at both stops, so the maps are taken to
be the same.

*Equal to the trace, step 10 (return of step 10):*

- The trace asks function 7 of all 382 objects, in registration order.
- The 62 objects whose frame storage it moves are the 62 that have a
  function 7 tree here, in the same order. They are also the 62 whose last
  result changes.
- No data word changes in the trace, and the first pass changes none here.
  So the room worked out for each of the 382 objects is the room the
  house saved. 234 of them stand on the lot.
- All the fields compared after step 9 (see the entry below), for all 382
  objects, with the last result. The globals, both game random streams,
  the removal queue and the arrays.
- Whether a room is outdoors, for the 8 rooms the trace has worked it out
  for by then. Those are the rooms the 62 trees asked about.

*Equal to the trace, the room count (entry of step 11):*

- For all 11 rooms: the half tiles, the covered half tiles, the walls
  that can take a covering, the walls that have one, and whether the room
  is outdoors.
- Global 27, which goes from 0 to 204.
- Everything compared at the return of step 10.

*Equal to the trace, steps 11 and 12:* the same fields after each. The
removal queue goes from four IDs to none. In step 12 global 9 goes from 0
to 5, global 13 and the funds from 2,515 to 2,542; the selected family's
ID is 5, its magic history is marked as read and its six words are zero.
The trace's next stop, the entry of step 13, differs from that one in
nothing compared here.

The comparison is not empty. Between the stops the trace changes exactly
what we change: the rooms, 62 last results, global 27, the queue, and
globals 9 and 13 with the funds.

*Consistent with the old prototype's isolated comparisons only.* The
prototype ran the original's code alone, outside a running game, on
made-up inputs. These rest on that and on nothing in the trace:

- The room of an object on a split tile. No object of the Goth house
  stands on one. The prototype compared 32,780 positions.
- Which room each triangle of a split tile has. The trace's memory does
  not hold the records. The prototype compared 80 made-up layouts and
  six real floors.
- The edge of a pool as a room boundary. The Goth house has no pool.
- The parts of the flag rules this house does not reach. It does have
  sloped ground, fences, a fence on a diagonal, and an upper level with
  supported and unsupported tiles, and its flags are equal in every
  cell. The prototype compared 96 made-up cases.
- The refused cases: crossing diagonals, unmirrored walls, a diagonal on
  the border.

What was wrong, and is fixed. `World::read_arrays` keeps the lot's own
cells outside the lot's size, and a lot with no array had zeros there.
That was marked unverified. The trace's memory at the entry of step 10
holds most cells of every array, also outside the size, and all of them
at some stop for the walls. Every held cell equals ours but one kind: the
ground level's wall cells outside the 52 tiles. In the original each is
`00 0f 01 00 01 00` and eight zero bytes: no segments, flags 0x0f, both
styles 1. The held lot has that in every ground wall cell when the load
starts (the trace's memory at the entry of the resize), and the house
file saved the same bytes there. A lot here has no arrays before the
load, so `read_arrays` now gives the ground level's walls that cell
outside the size (`BARE_GROUND_WALL`). After the fix every held cell of
every array is equal. The digests of the arrays and of the states after
steps 6e and 9 changed with it. The gated tests show that zeroing those
cells again gives the digests pinned before.

The same memory settles two things the entry below left unverified. The
eight grid cells outside the lot's size that name objects standing
elsewhere are still there in the trace after step 9, as they are here. And
the cells outside the size of the other arrays are what we have.

What did not match. Object 19's second frame has tree ID 0x8118 in the
trace and 0x118 here, at all five stops, as before. Nothing in this work
explains it. The C runtime's random stream and six type attribute words
belong to the game that was running; with those copied from the trace's
start of the load they are equal at every stop.

What is **unverified**:

- **The listed tiles of rooms 0, 3 and 1024.** The trace holds how many
  there are, not which.
- **The split tile records**, beyond their number.
- **An object on a split tile**, as above.
- **Room fields 2 and 3 as a script reads them** (level and area). The
  trees of step 10 read only field 1. The area a script reads before the
  rooms are counted is 0 here, because the count is; no tree shows that.
- **Global 10 in step 12.** The house saved its own number there.
- **Selecting no family**, and a family record older than version 9,
  which has no magic words. `sims boot` refuses a house no family lives
  in.
- **The level argument of step 10.** The original can run both passes for
  one level only. We always run both levels.
- **Arrays 4, 6 and 7**, as before.

What is not modeled, and not compared:

- The rest of the original's room update: what each object adds to its
  room, the rooms' light and scores, the lot's outdoor score, the lighting
  of the tiles. In the trace this update changes no object, random stream
  or global but 27, so the state we compare is not affected yet.
- Which rooms have people in them. The first pass of step 10 tells a
  person's room that it is occupied.
- The rooms of the bare lot before the load. The original has two after
  the resize (step 3). `World::rooms` is empty until step 9a. No script
  that runs in between reads a room.
- Two booleans about the upper level, the roofs, and the redrawing.

What fails loudly: two diagonal walls crossing in one tile; a wall its
neighbour does not mirror; an enclosing diagonal wall on the border of
the arrays; more than 1,024 rooms on a level; a missing or misshapen
array; asking for a room before the rooms exist; a room field other than
1 to 3; Refresh modes 1 and 2 on an object that is in a room; an object
that removes itself while it is told the rooms changed; and booting a
house no family lives in.

What the trace shows about step 13, noted here because it sets the next
piece of work. The step asks function 2 of all 379 objects that are not
people. 184 objects have a changed type, and 158 of them have no Reset
tree, so they are taken off the lot and put back: the trace asks
functions 10 and 9 of each, and of every tile of its group. 148 of those
stand on the lot. So step 13 needs placement: taking an object off its
tile and putting it back, with the grid, the slots, containers and
groups. A trial run of the Load trees here also stopped at Notify Stack
Object out of Idle, Generic Sims Calls 11 and 43, and Refresh mode 2.
`docs/roadmap.md` has the numbers.

What would reopen this: a house with crossing diagonals or with an object
on a split tile; a capture that holds the split records or the tile lists
of the big rooms; a script that reads a room's area before the rooms are
counted; anything that reads a room's light or score, which needs the
rest of the room update.

## 2026-11-13: The controllers are reconciled among the saved objects, and the grid repair clears only what is inside the lot

Load step 8 makes the lot hold one of every lot-wide controller, now that
the saved objects are on it. Load step 9 clears cells of the ground
level's object grid that name an object which is not there. `sims boot`
runs both and stops before step 10.

What was decided:

- **Step 8 is the reconciliation of step 3, called on the full lot**
  (`Interpreter::reconcile_controllers`). Creating the missing controllers
  needed no new code: the factory, the primitives and the services the
  141 controllers reached on a bare lot were enough for the 137 created
  here.
- **Later duplicates are removed.** The original lists the objects whose
  definition has a nonzero global word once, in registration order. For
  each controller definition it keeps the first listed instance and
  removes each later one at once (`Interpreter::remove_now`), scanning the
  list again from the start after each removal. The function now returns
  what it created and what it removed (`Reconciled`), and so does
  `Interpreter::resize`.
- **An object can be deleted on a lot with people, if no person refers to
  it.** The original tells every person about a deleted object. A person
  that does not refer to it changes nothing. A person refers to an object
  when it is using it, or when its running, last or a queued action
  targets it or shows its icon. Deleting an object a person refers to
  fails with an error that names the person, before anything changes.
- **Step 9 is a step of `World`** (`World::repair_grid`), because it runs
  no script. It looks only at the ground level's grid, and only at cells
  inside the lot's size. A cell is cleared if its ID is negative, names no
  object, or names an object whose tile and level are not that cell on
  level 1. The tile is the position divided by 16, rounding down
  (`World::tile`).
- **"Restore placement links" is not a step.** The roadmap listed it with
  step 9, after the old prototype, which built its own link tables at
  this point. The original's function for steps 8 and 9 (0x43fc02) ends
  after the grid repair. The links are state the object read already
  filled: the grid, and each object's slots. Nothing is rebuilt.
- **Step 11 has its state change** (`World::discard_removals`). It is not
  called, because step 10 is not ported.
- **The snapshot format stays 5.** `World` has no new field.

How it was checked. The old prototype kept, privately, a trace of the
original loading the Goth house, with the game stopped at several moments.
One is the return of 0x43fc02, after steps 8 and 9. Our state after both
steps was compared with it outside the repository.

Equal for all 382 objects: the ID and GUID, the 72 data words, the temps,
the attributes and their number, the scheduler word, the last result,
each frame's node, stack object, arguments, locals and saved primitive
state, the relationships, the number of slots, the position, the tile and
level bytes the grid repair reads, and the mark of an object that asked to
be removed without resets. Equal for the three people: the 256 words of
person data. Also equal: the registration order, the tick, the lot size,
the 64 globals, both game random streams, the four IDs queued for removal
(42, 44, 278, 280), the scheduler cursor, the funds, the indicator
references, the one instance list, and inside the lot's 52 tiles the
terrain heights, both levels' floors, walls, object grid and flags, the
pool and the water.

The comparison is not empty. Between the stop after the object read and
this one, the trace gains 137 objects and two queued IDs and its global
random stream moves. Nothing else it holds changes: no saved object, no
global, no type attribute, no grid cell. The same is true here.

*Slots.* The trace also saved pages of the game's memory at each stop.
They hold the slots of 198 objects at this stop and of 149 saved objects
at the stop before, 216 objects in all. For each, the object ID in every
slot equals ours. Those objects hold 30 of the 32 links between objects
that our state has.

*The grid.* In the trace every occupied cell of both grids names an
object whose tile bytes are that cell, before and after step 9, so the
original cleared nothing. We clear nothing either. The saved grid does
name eight objects that are elsewhere, all in cells outside the lot's 52
tiles; scanning the whole 64 would clear them. That the scan stops at the
lot's size is from the original's code, which was read for this port, and
from the old prototype, which ran that code alone on made-up grids with a
size smaller than the array.

What did not match. Object 19's second frame still has tree ID 0x8118 in
the trace and 0x118 here, as after the object read. Nothing in steps 8 or
9 touches it. In the trace the frame is gone after step 13, when the
object's thread holds only its Main frame. It is the only frame with that
bit at any stop looked at. It stays unexplained.

The C runtime's random stream and six type attribute words belong to the
game that was running, as before. With those copied from the trace's
state at the start of the load, they are equal at this stop too. Step 8
draws nothing from the C runtime's stream.

What is **unverified**:

- **Removing a duplicate controller.** The Goth house has none, so no
  capture shows it. The selection follows the old prototype, which
  compared its loop with the original's 0x442b9f run alone on 97 made-up
  catalogs, with the factory and the removal replaced by stubs.
- **That a person who does not refer to a deleted object changes
  nothing.** From the old prototype, which compared its port of the
  person's notification with the original's code run alone. No capture
  holds a deletion on a lot with people. Which of a person's fields count
  as a reference follows that port too; where it tests the frames of the
  person's thread that use an object, we test the saved list of uses.
- **Clearing a grid cell.** No capture shows one being cleared. The rule
  is from the original's code and the old prototype's isolated runs of it.
- **Cells outside the lot's size.** The trace does not hold them. We keep
  what the house saved there.
- **The slots of the other 166 objects**, and every slot field but the
  object ID.
- **`World::discard_removals` in its place.** The trace shows the queue
  going from four IDs to none with no object changing. Our state at that
  moment was not compared, because the load does not reach it.

What is not modeled and fails loudly instead: removing a duplicate that
is on the lot, holds or sits in something, is multi-tile, or has a
callback for leaving the lot; removing one a person refers to; a
controller that a script removed while the controllers were being
reconciled, where the original would use freed memory; and a ground grid
that is missing or not 64 by 64 two-byte cells.

What is not modeled and not compared: the animation tester's ID, which
step 8 notes for the interface; the log the second pass of step 9 writes;
and two flag bytes the object registry sets around these steps.

What the trace shows about the next steps, noted here because it changes
the plan. Step 10 needs the lot's rooms, which the original builds
between step 9 and step 10 (the trace has 2 rooms before and 11 after).
Working rooms out from the walls is the next piece to port. And the two
IDs step 3 left in the removal queue, 42 and 44, name saved objects after
step 6e. Step 11 empties the queue before anything acts on it, so it must
not be skipped.

What would reopen this: a house that saved a controller twice; a house
whose grid names a missing object inside the lot's size; a capture of
either; or finding what sets bit 15 of a frame's tree ID.

## 2026-11-13: Saved objects are constructed and then read, and the load runs through the object read

Load step 6 reads the rest of the house onto the cleared lot. This entry
covers its five parts: the lot arrays, the floor and wall renumbering, the
type table, the second clear and the saved objects. `sims boot` now runs
all of them and stops before step 8.

What was decided:

- **Saved floors and walls are widened into the lot's arrays**
  (`World::read_arrays`). A house of version 53 to 62 saves a floor cell
  as one byte and a wall cell as 8 bytes. The lot holds 2 and 14. The
  original widens only the cells inside the lot's size. Here the cells
  outside it are zero when the lot had no array. Other house versions
  fail.
- **The renumbering is ported for an empty map only**
  (`World::renumber_surfaces`). A floor ID the map should have listed
  becomes 65535. A wall cell that would change fails. A map with entries
  fails.
- **Reading the objects is a step of the interpreter**
  (`Interpreter::read_objects` in `crates/sims-vm/src/objects.rs`). It
  needs the object definitions, which `sims-sim` does not hold.
  `World::read_objects` is gone. `World::from_house` stays as a way to look
  at a house file; it copies saved fields and constructs nothing.
- **A saved object is constructed first.** It gets the attributes its
  definition asks for (8 at least), a slot for its tile and one for each
  object it can hold, and its definition's dynamic sprite flags. The saved
  fields are read over that and never shrink it. A record with no
  scheduler word keeps the word registration gave.
- **Multi-tile links are state** (`World::next_parts`). Each tile joins its
  lead tile's list in the file's order.
- **A person's constructor is ported as far as it changes state we hold.**
  It reads the skeleton's file if the game has not read it yet, builds the
  skeleton, attaches the person to its neighborhood record, and makes
  default interests and personality if the record gives it none. Each of
  those draws from the C runtime's random stream. The saved person data
  replaces the defaults. Person data has 256 words; an older save's 80
  are followed by zeros.
- **Which skeleton files the game has read is state**
  (`World::skeleton_files`). It belongs to the running game, like the C
  runtime's stream.
- **`sims_content::People` finds what the constructor needs.** The
  skeleton's name is the first of the person type's body strings. The
  file is the one skeleton file that defines that name, among the
  archives in `GameData/Animation` and the indexed object directories. A
  name that two files define is an error.
- **The snapshot format is 5.** The two new fields are at the end of the
  encoding.

How it was checked. The old prototype kept, privately, a trace of the
original loading the Goth house, with the game stopped at several moments.
Three of them were compared with our state outside the repository. A
"capture" below is one of these stops.

*After the arrays and the renumbering* (the capture at the entry of
0x43fc02). The capture holds the cells inside the lot's 52 tiles of the
terrain heights, the floors, walls, object grid and flags of both levels,
the pool and the water. All equal ours. The capture taken before the
arrays are read differs from the house, so the comparison is not empty.
The house's two maps are empty, and none of its cells holds an ID the
renumbering covers, so the renumbering changes nothing here; the capture
agrees.

*After the object read* (the capture at the return of 0x449c5e). Equal for
all 245 objects: the ID and GUID, the 72 data words, the temps, the
attributes and their number, the scheduler word, the last result, each
frame's node, stack object, arguments, locals and saved primitive state,
the relationships, the number of slots, and the position. Equal for the
three people: the 256 words of person data. Also equal: the registration
order, the tick, the lot size, the 64 globals, both game random streams,
the two IDs queued for removal, the scheduler cursor, the funds, the
indicator references, the one instance list, and the arrays again.

One object shows why the constructor matters: its definition asks for 16
attributes and the save holds 11. The capture has 16.

*The random draws.* The C runtime's stream starts in a state that belongs
to the game that was running. With that state, and the six type attribute
words that also belong to it, copied from the capture at the start of the
load, our stream after the object read equals the capture's. So the
constructors of the three Goths draw as many numbers here as in the
original. The stream's state only depends on how many numbers were drawn,
so this does not show what each draw was for. In the capture the stream
does not move anywhere else between the start of the load and the end of
the object read.

*Multi-tile links.* The trace does not hold them. Another capture, of the
same house four ticks after its load, does: all 142 tiles name the same
next tile, lead tile, origin and offset as ours.

What did not match. One frame of object 19 has tree ID 0x8118 in the
capture and 0x118 in the save and here. Object 17 has the same type and
the same thread, and matches. The capture still shows the bit after
step 8. What sets it is not known. It is **unverified** whether it is
state.

What is **unverified**:

- **Arrays 4, 6 and 7, and every cell outside the lot's size.** The
  captures do not hold them. That a lot without an array has zeros outside
  its size is a guess.
- **The rewrite to 65535.** The Goth house has no cell it applies to. It
  follows the old prototype, which compared its port with the original's
  code run alone on 24 made-up lots.
- **What the draws are for.** The split into skeleton files, skeletons
  and defaults follows the old prototype, which compared each part with
  the original's code run alone. Only the total was compared with the
  running game.
- **That the game has read no skeleton file before its first load.** The
  total fits that for this capture. Another session might have read one
  already.
- **Which files the skeleton lookup searches.** The original uses a
  registry of avatar files. Here the name must be defined by exactly one
  file.
- **Footprints, the short after the position, the frame arena size, slot
  contents, sprite flags, motives and queues.** They are the saved values.
  The captures do not hold them, or were not decoded for them.
- **The other 110 changed flags and the rebuilt type table**, as before.

What is not modeled and fails loudly instead: a house version outside 53
to 62; a floor or wall map with entries; a wall cell the renumbering
would change; an object with graphics of its own; a person with no
neighborhood record, for which the original makes one; a skeleton that no
file or two files define; saved attributes beyond what the constructor
allocates; and reading objects onto a lot that holds objects.

What is not modeled and not compared: the lot's lighting, which the array
read resets; object flags; a person's animator, saved animations and
built-in slot positions; and the game's count of queued interactions.

How the earlier states were kept. The states pinned for the resize and
the first clear are the old encodings plus the two bytes of the new,
empty fields. The gated tests check that: without those bytes each
hashes to its format 4 digest.

What would reopen this: a house with custom floors or walls; a house
whose people have no neighborhood record; a capture that holds the cells
outside the lot's size; or finding what sets bit 15 of a frame's tree ID.

## 2026-11-13: Clearing the lot deletes objects one at a time, and the type table is read before the objects

Load step 5 removes every object the lot holds before the house's own
objects are read. Removing is not dropping: each object leaves the lot,
resets one last time, and is then unlinked. The reset runs the object's
Reset tree, a script, so the clear can change globals, draw random numbers
and queue removals. This entry also corrects the order of steps 6 and 7 in
`docs/roadmap.md` and says where the load now stops.

What was decided:

- **Deletion is ported for the simple case** (`crates/sims-vm/src/removal.rs`).
  `Interpreter::remove_now` takes an object off the lot and deletes it.
  `Interpreter::delete` releases the ownership of interface events, runs
  the last reset, and unlinks the object from the registration list and
  from its type's instance list. `Interpreter::reset` runs the Reset tree
  on top of the object's thread and then restarts the thread at Main; an
  object with no Reset tree only leaves the lot. `Interpreter::clear` is
  step 5: the two indicators first, then the head of the registration list
  until it is empty.
- **The removal queue keeps IDs whose objects are gone.** The original's
  queue holds bare IDs. The clear does not touch it, and the load erases
  it later (step 11). So after the clear `World::removals` still names 42
  and 44.
- **An instance list outlives its instances.** A type's list in
  `World::instances` stays, empty, when its last object is deleted. An
  object read or created later is appended to it.
- **`World` holds the indicators' IDs** (`World::indicators`, 0 for none).
  The clear needs to know which objects they are, and it sets both to 0.
- **The changed flag is state of the running game.**
  `World::changed_types` holds the GUIDs of the types whose scripts
  changed since a house that listed them was saved. The original keeps
  the flag on each definition. Reading a type table sets or clears it for
  the types the table lists and leaves the others alone.
  `Interpreter::read_types` does that and rebuilds `World::types`.
- **Steps 6 and 7 run in another order than the roadmap said.** After the
  clear the original reads the lot arrays, renumbers the saved floors and
  walls, reads the type table and marks the changed types in the same
  pass, clears the lot a second time, and only then reads the objects. The
  roadmap had the changed types after the objects. The step table now
  lists 6a to 6e.
- **`World::from_house` is split.** `World::read_arrays` and
  `World::read_objects` read onto the lot the load holds and keep what the
  earlier steps left. `from_house` stays as a way to look at a house file.
- **`sims boot` stops after the lot arrays.** The next thing the original
  does, renumbering floors and walls, is not ported.
- **The snapshot format is 4.** The two new fields are at the end of the
  encoding.

How it was checked. The old prototype's harness captured the original at
the moment the clear returns, loading the Goth house. Our state was
compared with that capture outside the repository. Equal: no objects, an
empty registration list, the tick, the lot size, the 64 globals, both game
random streams, the queued removals (42 and 44), the scheduler cursor, the
funds, both indicator references (0), the one instance list (empty), and
the type attribute records' GUIDs, order and lengths. Eighteen controllers
ran a Reset tree on the way. The test
`the_goth_house_clear_removes_every_object` pins a digest of our state.

The state when the resize returns was not compared again. Its encoding is
the old one plus the three bytes of the new fields, and without those
bytes it hashes to the digest the earlier comparison pinned.

For the type table, a capture taken after the original had read it shows
the three Goths' own types: an Init signature of 20 for each, the changed
flag set, and a Main signature never worked out. We get 20 and the flag
for all three. That also confirms the rule that Main is only looked at
when Init matches.

What did not match, and why. The same things as when the resize returns,
and for the same reason: the C runtime's random stream and six type
attribute words belong to the game that was running. With those copied
from the capture into the lot before the resize, they match after the
clear too, so no Reset tree changes either one here.

What is **unverified**:

- **The lot arrays** (`World::read_arrays`). No capture covers the
  arrays after they are read. Which arrays are read, and in what order,
  comes from the old prototype's reading of the code. A missing array is
  an error here; the original stops reading arrays and carries on.
- **The other 110 changed flags and the rebuilt type table.** Ported from
  the old prototype, which compared its version with the original's code
  run alone on made-up records.
- **`World::read_objects`.** No capture covers the object read. It copies
  saved fields and does not construct anything, so it is not a load step
  yet and `sims boot` does not call it.
- **Deletion paths the 143 objects do not take.** A Reset entry whose
  tree is missing, a Reset tree that removes or creates objects, and an
  object that owns interface events are ported from the old prototype and
  covered by synthetic tests only.
- **The reset outside a deletion.** `Interpreter::reset` with a Reset
  tree is the same code path, but nothing calls it that way yet.
- **The leftover type table.** After the clear `World::types` still lists
  the controllers' types. That table is our bookkeeping, and the type
  table read replaces it.

What is not modeled and fails loudly instead: removing an object that is
on the lot, in a container, holding something, multi-tile, a person, or
has a callback for leaving the lot; deleting anything while a person is on
the lot, because people must drop their references to it; resetting an
object with no Reset tree outside a deletion, which rebuilds and replaces
it; reading a type table onto a lot that holds objects; a lot array that
is missing or not 64 by 64; and a house with a GUID translation table.

What is not modeled and not compared: the interface being switched off
during the clear, the objects' graphics, the pools of spare indicators,
the redraw of the room a removed light was in, the deletion mark on the
object (only a person's reset reads it), type ID 0 in the rebuilt table,
and type tables older than version 2.

What would reopen this: a capture taken after the arrays or the objects
are read; a lot whose objects are on tiles when it is cleared; or a house
with custom floors or walls, which makes the renumbering matter.

## 2026-11-13: A load starts from the lot the game holds, and resizing it creates the controllers

The original keeps one lot for as long as it runs and loads each house into
it. Before it reads a house's objects it reads the house's simulator state,
resizes the lot to the house's size, and that resize creates every lot-wide
controller the lot lacks. It then removes every object and only then reads
the saved ones. So 141 controllers run their Init and first turn on a
nearly empty lot, draw random numbers and write globals, and are thrown
away. The load sequence in `docs/roadmap.md` now starts with these steps.
This replaces the ten-step sequence of "The lot load follows a written
sequence" below, which read the whole house first, and that entry's mode 0.

What was decided:

- **`World` is the lot the game holds, not a house.** `World::empty` is the
  lot before any load, `World::read_simulator` reads a house's simulator
  state into it, and `Interpreter::resize` shrinks it. `World::from_house`
  still reads a whole file at once; it is no longer the load path and will
  be split when the saved objects are read onto the cleared lot.
- **The object factory is ported** (`Interpreter::create`): lowest unused
  ID, head of the registration list, the constructor's storage, the date
  stamp, the reset, Main installed, Init run synchronously above it, one
  first turn, then the scheduler flags and the price.
- **Sleep yields.** An object that is not a person and not locked out
  hands its count to the scheduler, which is the low half of the registry
  word. `Interpreter::turn` runs a thread until it yields or returns.
  Resolving data word 25 wakes an object, as in the original.
- **New state.** `World` gains the C runtime's random stream, the lot's
  size, the objects waiting for removal, the type attributes, the per-type
  instance lists, the owner of interface events and the selected family's
  magic history. `Object` gains its last result and a removal flag. The
  snapshot format is 3.
- **Frames of new objects need a type ID.** A frame names its code by the
  lot's type table, which the original only builds when it saves. A type
  the table lacks takes the lowest unused ID. This is our bookkeeping; no
  original state corresponds to it.
- **The whole catalog is linked.** The resize walks every definition, so
  `sims boot` links them all. `Program` also holds each file's interaction
  tables and slot counts, the career list from `work.iff`, and the
  neighborhood's lot zoning.
- **The context's mode is 1.** `apply_context` no longer clears the family
  in global 9; `World::clear_family` does, after the resize, because the
  controllers must still see the saved family.

How it was checked. The old prototype's harness captured the original as it
loaded the Goth house, at the moment the resize returns. Our state was
compared with that capture outside the repository. All 141 controllers
match: GUID, the 72 data words, temps, attributes, last result, every
frame, the registry word and the position. So do the 64 globals, both game
random streams, the tick, the lot size, the registration order, the two
controllers waiting for removal, the scheduler cursor, the funds, the
instance lists, and the type attribute records' GUIDs, order and lengths.
The test `the_goth_house_resize_creates_the_controllers` pins a digest of
our state and runs when `SIMS_INSTALL` is set. The capture is private.

The mode is 1 on the capture's evidence: at the start of the resize, before
any script has run, global 15 is 1 while the house saved 0.

What did not match, and why. Three things in the capture belong to the game
that was running, not to the house or the install:

- The two indicator objects. Ours are created on an empty lot whose clock
  is zero, and have just gone to sleep. The capture's carry another
  creation date and had been asleep for a while.
- The C runtime's random stream, which no file holds.
- Six type attribute words, one for each help system, which are set in
  the capture and clear in the neighborhood file.

With those copied from the capture's state at the start of the resize, the
two indicators match as well, which makes all 143 objects. The controllers
match either way.

What is **unverified**:

- **The starting lot.** The creation date and time the capture's
  indicators carry is exactly the clock saved in `Houses/House00.iff`, the
  empty lot every neighborhood ships with. So the original most likely
  loads House00 when it starts, and the indicators come from that load. We do not model it: `sims boot`
  starts from `World::empty`. This cannot change what a load leaves behind
  unless a controller reads the indicators' state, and none did here.
- **The six type attribute words.** Something in the running game set
  them before the load. We do not know what. The 141 controllers end in
  the same state with either value, but later ticks may not.
- **The C runtime stream's starting state.** It is 1, the runtime's own
  default. Nothing in this step draws from it.
- **Fields the capture does not hold.** A new object's footprint, the short
  after it, its frame arena size, its relationship flag and its slots are
  set to what the original's saves hold for such an object.
- **Paths the 141 controllers do not take.** Sleep, waking, Remove Object
  Instance, Generic Sims Call 28 and 43, event ownership, Set To Next
  modes 4 and 10 and the type attribute scopes are ported from the old
  prototype. The capture confirms the paths these controllers take through
  them, not the others. The original keeps an object with a headline
  showing out of the scheduler's hands while it sleeps; headlines are not
  modeled, so that test is missing.
- **The turn budget.** Init and the first turn may each run 100,000
  instructions, the old prototype's guard. The original's limit is not
  known.
- **Which zoning file the original reads.** We read the neighborhood's
  `LotZoning.iff`; the old prototype read the install's
  `MasterLotZoning.iff`. Both give the same table for the shipped
  neighborhoods.
- **Global 25 and free will in the context.** The capture agrees with
  them, but the house saved the same values, so it proves nothing.

What is not modeled and fails loudly instead: growing a lot; resizing a lot
that has arrays or objects standing on it; a second instance of a
controller, which needs deletion; removing an object that is on the lot,
holds or sits in something, is a person, or has a callback for leaving the
lot; the factories for people, cursors and multi-tile objects; Generic Sims
Call routines other than 28 and 43, and 43 once a family's magic history is
held; a type that wants its attributes cleared for each lot.

What is not modeled and not compared: the lot's rooms and their tile
counts, the interface's action counter, and the interface's lists of
indicators. The resize only writes the indoor area of a bare lot, which is
0, to global 27.

What would reopen this: a capture of the original at startup, which would
settle the starting lot and the type attribute words; a house whose
controllers take the unverified paths; or a load into a lot that is not
bare.

## 2026-11-13: The catalog follows the original's scan order and file tables

Objects register in the order the original scans for them (0x43c609):
`GameData/Objects`, `GameData/UserObjects`, the neighborhood's
`Characters`, `Downloads`, the packs in bit order up to Superstar, the
shared directories, and Makin' Magic last. `Index::add_characters` puts
the people in that place. This replaces "characters register after the
install" below.

Within a directory the original does not use the file system's order. It
keeps each directory's entries in a hash table keyed by lower-case name and
walks the table (0x5ba9bd). `sims-content`'s `file_table` reproduces the
hash, the bucket counts and the growth rule, ported from the old prototype.
An archive's members are entries of the directory the archive is in, and a
member whose name holds a path goes into that subdirectory, whether or not
it exists on disk. The first entry of a name wins, except that a file on
disk replaces an archive member. This replaces "directory order is NTFS
name order" below.

It is checked against the original. When the Complete Collection loads a
lot, it creates the lot-wide controllers the lot lacks by walking its
catalog. A capture from the old prototype's harness holds the 141 objects
that made, in order. Picking the same definitions from our catalog gives
the same 141 GUIDs in the same order. The test
`the_catalog_lists_controllers_in_the_originals_order` pins a digest of
that order and runs when `SIMS_INSTALL` is set. The capture itself is
private.

What that leaves **unverified**:

- The order of two names that share a bucket, and which of two archives
  supplies a member both hold. Both depend on the order Windows lists a
  directory in, for which `Index` uses NTFS name order. The 141 controllers
  show no such case.
- Where the people fall. No controller is a person, so their place rests on
  the old prototype's reading of the scan.
- Houses saved by the original list their object types in catalog order,
  and those lists disagree with the capture in one way: they show larger
  tables for some directories (389 buckets for the base game's objects,
  not 193). The studio's machines presumably had more files in those
  directories. The capture is from the install we support, so it wins.

## 2026-11-13: The lot load follows a written sequence and stops at the first gap

The load sequence in `docs/roadmap.md` lists the ten steps the original runs between
reading a house file and the first tick, with what `sims boot` does for
each. `sims boot` runs them in order and stops at the first step that is
not implemented, so the hash it prints is always of a state the original
passes through. The alternative, running whatever callbacks already work,
would give a state the original is never in, and its hash would mean
nothing to the comparison harness.

The first step added is the globals the running game supplies
(`World::apply_context`). The values for a Complete Collection install
(mode 0, global 25 zero, free will on, expansion mask 0xff, performance
class 2) are the old prototype's headless profile and are **unverified**.
The performance class comes from the machine's memory and CPU speed, so
the comparison harness must read all five from the running original.

Writing the sequence down showed that the catalog order was wrong for load
step 4, where new objects get IDs in catalog order. The entry above fixes
it.

## 2026-11-13: Init primitives stop at state the lot does not hold

Test Object Type, Set To Next, Refresh, Play Sound Event and the stack
object's definition words (scope 21) are ported from the old prototype.
`Program` now keeps every OBJD as the 108 words the original holds after
registration: versions 136 and 137 become 138, and every record is cut or
zero-padded to 216 bytes (0x431759), so every scope 21 index under 108
reads. Definitions are listed in the file's native resource order, as the
catalog registers them; the prototype used physical file order.

Refresh mode 0 only redraws, so it does nothing here. Modes 1 and 2 mark
the object's room dirty, which makes the next room update rebuild the
room's score and lighting. Rooms are not modeled, so those modes fail
loudly unless the object is in no room (data word 29 is -5). This is now
the most common init failure on the Goth house. Play Sound Event only
queues audio, so it takes the true branch and changes nothing.

Set To Next walks registration order (modes 0, 2 and 6), people (1),
multi-tile groups (3) and families (12). Mode 4 fails loudly: the
original keeps a per-GUID list of instances for the whole process. The
list is filled in ID order on first use and appended to as objects
register, even across lot loads, and `World` has no such list. Modes 5
and 7 to 11, and unknown modes, also fail loudly. Three choices are
**unverified**:

- The person vector is the people in registration order. That is how
  loading builds it; Sims joining or leaving may change it.
- A multi-tile group's chain is its head, then its tiles in registration
  order, which is how loading links them (0x423223). `World` keeps only
  each tile's head, so splitting or joining groups at run time would need
  the links themselves.
- Test Object Type on a tile whose group has no catalog master, and Set
  To Next mode 6 in a tree with no parameters, fail loudly. What the
  original does there is not known.

The three load callbacks that fail with "tree 0x100b is missing" are not a
linking bug. `FlowersOutdoor.iff` OBJD 16808 has no OBJf, so its callbacks
come from the OBJD itself, and its load field names 0x100b. The file has
no such tree and no semi-global file. The prototype found that the
original skips a callback whose tree is absent (0x4749b5). Whatever runs
callbacks will have to do the same.

## 2026-11-13: The interpreter runs on the saved frames

An object's thread is its stack of saved frames in `World`, so the
interpreter keeps no state of its own and every snapshot holds a resumable
stack. Frames name their code owner by objt type ID, so `World` now keeps
the lot's type table (`types`, type ID to GUID; snapshot format 2).
Expression, Random Number and the operand scopes are ported from the old
prototype with its native error codes. Scopes over state `World` lacks
(tree advertisements, definition words, rooms, neighbors, jobs,
neighborhood data, type attributes) and every other primitive fail loudly.
Four semantics are **unverified**:

- New frames start with zeroed locals. The original reuses a frame arena,
  so locals inherit words left by earlier frames; saves carry no arena.
- Random Number draws from a stream the caller names. The original swaps
  the lot stream in only around the lot update (0x46d553).
- Motive reads round half to even (the x87 default); the prototype rounded
  half away from zero.
- Motive arithmetic rounds once from the x87's 64-bit significand to 32
  bits. The FPU precision the original runs it at is not known.

## 2026-11-13: Linking is checked against saved behavior signatures

Each object type in a save records two signatures: for its init and main
trees, the root's revision word shifted left 16 plus the revisions of every
tree reachable from the root (0x4026ed, recovered by the old prototype).
`sims boot` recomputes them from the linked program. Across 296 shipped
houses, 37,655 of 47,359 saved types match. In the best houses nearly all
match, and the misses differ by a revision or two, as when a later pack
revised a tree after the house was saved. A linking mistake would give
unrelated numbers.

On the Goth house only 36 of 113 types match: it was saved by the base
game, and the expansions revised most of its objects' code. The original
resets a saved object's script state when its signatures differ
(0x43a516), so booting this house exercises that reset for 77 types. The
interpreter has to implement it before the first tick.

## 2026-11-13: The Complete Collection ISO is the supported install

Decided by the maintainer. The vision named the Legacy Collection as the
primary install; it now names The Sims Complete Collection, installed from
its ISO, and leaves other editions out of scope. That is the install the
emulator image already holds and every check so far ran against, so no
code changes. It replaces the entry below on recognizing only the CD
layout: that is no longer a gap.

## 2026-11-13: The state hash is SHA-256 of the postcard encoding

`World` is encoded with postcard: fields in declaration order, integers as
little-endian varints, floats as raw bits (NaN payloads and negative zero
kept), and maps in key order. Nothing in it depends on the platform, and a
pinned-hash test runs on both CI platforms. Snapshots store the same bytes
behind a 4-byte format number, in a REDB table keyed by name. Changing the
encoding means bumping `snapshot::FORMAT` and the pinned hash. Comparison
with the original goes field by field, not through this hash: the hash only
proves that our own runs repeat.

`World` keeps every field the readers decode for objects, the simulator
and the lot arrays. It leaves out bytes nobody understands yet (format
tails, raw ObjM records), the HOUS view and model, and the wall and floor
chunks. They join when the interpreter or the comparison needs them.

## 2026-11-13: The target house is the Goth house

The boot goal needs one house. It is `House05.iff` in the base game's
shipped neighborhood: the Goth family (Mortimer, Bella and Cassandra), 245
saved objects, SIMI and ObjM version 0x3e. The installed copy is byte for
byte the shipped one, so the emulator can load it without copying files in.
The Newbie house (`House07.iff`) is smaller but was played and saved by the
old harness, so the installed copy differs from the shipped one. Reopen if
the Goth house needs something the harness cannot capture.

## 2026-11-13: Placeholder objects carry no registry word

Every ObjM record ended with a field-encoded registry word, until the
expansion houses: the `*Placeholder` object (GUID 0x3C08965F) never has one.
Across 139,000 ordinary records in the installed and shipped houses, 24
to 31 bits remain after the dynamic sprites. Across 27,000 placeholder
records, 0 to 7 remain, so no field fits. The reader now reads the word only
when at least a byte of the record remains, and stores None otherwise.
Why the original writes no word is unknown; it may skip objects missing
from the scheduler's registry. Unverified until the harness compares a
placeholder.

## 2026-11-13: Neighborhood characters register after the install

Superseded by "The catalog follows the original's scan order and file
tables" above: characters register after the user objects.

A house's Sims are defined by files in the neighborhood's `Characters`
directory. `Index::add_directory` appends that directory as one more mount,
after Downloads, so the catalog registers characters last. Where the
original registers them is unverified. A family with house number 0 has no
house: in the shipped and installed neighborhoods every such family lacks
flag bit 0, and every family with it has a nonzero house.

## 2026-11-13: Some installed houses do not load yet, on purpose

`sims houses` loads 660 of the 718 house files in the install's eight
neighborhoods and the shipped one. The 58 failures:

- 18 are `House00` and `House11` in each neighborhood: base-game templates
  with ObjM versions 0x33 and 0x38, whose record layout is not known yet.
- 40 are houses 56, 63, 69, 76 and 77, which hold Sims whose character files
  are not in the neighborhood: Superstar's community templates, and houses
  the old harness saved. The original presumably repairs or drops such residents,
  which we have not reverse engineered.

Neither blocks the target house. Reopen when a goal needs those houses.

## 2026-11-13: Later GUID registrations win; semi-globals resolve to the first match

The catalog registers object definitions in load order, and a GUID resolves
to its last registration. The old prototype recovered that rule from the
original's lookup. Multi-tile masters are the exception: the first
registered master wins, and that is for the interpreter to apply. A GLOB
link resolves to the first file of that name in load order. That part is
unverified: the Complete Collection has no duplicate semi-global names, so
nothing tells the two rules apart yet.

## 2026-11-13: Directory order is NTFS name order (unverified)

Superseded by "The catalog follows the original's scan order and file
tables" above: NTFS name order now decides only the order of names that
share a bucket.

The original lists each mounted directory with FindFirstFile, and that order
decides the object catalog order. On Windows/NTFS it is name order with case
folded. `Index::build` uses that order and searches a subdirectory where its
name falls among the files. Neither detail has been confirmed against the
original's walker yet. The old harness ran the game under Wine on ext4,
whose directory order can differ from NTFS. The comparison harness must
either record the order the original saw and feed it to us, or put the
install on a file system that lists in NTFS order.

## 2026-11-13: Expansions come from the pack directories

The original reads the installed packs from the registry (`EPxInstalled`,
`SIMS_GAME_EDITION`). We have no registry, so a pack counts as installed
when its directory holds a FAR archive. On the Complete Collection install
this gives edition 0xFF, the same as its registry.

## 2026-11-13: Only the CD layout is recognized for now

The CD layout is checked against a real Complete Collection install. The
Legacy Collection's layout is not known here, and a guessed table would not
be tested by anything. It is added when someone can list a real Legacy
Collection install.

## 2026-11-13: Readers stay strict on unreferenced bad chunks

21 TTAB chunks in the original install are version 3 or zero bytes long.
Nothing references them. The reader keeps rejecting them, and loaders
decode only the chunks they use. Reopen if a referenced chunk ever fails.

## 2026-11-13: SIMI versions 0x33 and 0x38 use the later layout (unverified)

Only the template houses carry these versions. Their fields up to the budget
read plausibly with the 0x3e layout, and 0x33's unread tail is 24 bytes
shorter. Check the fields against the original when the harness loads one
of these houses.

## 2026-11-13: SPR# versions 503 and 505 share the 502/504 frame encoding

All 14 such sets in the install decode exactly with it. What the version
number itself selects is unknown.
