# The lot's rooms are worked out from its walls, and the load runs through the household selection

*Decided 2026-11-13.*

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
