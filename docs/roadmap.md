# Roadmap

The current goal and how work toward it runs. `docs/vision.md` says what we
are building. This file says what we are building *next*, and when it counts
as done.

## Current goal: the simulation matches the original across what a real player does

The bar is the same as before: field for field against recordings of the
original run in the emulator, with every difference explained. What
widens is what is recorded: a whole game day, more houses, a player's
orders, every primitive the install uses, and an expansion-pack lot.

Done when, in strict mode (the default, unchanged):

- a recording of the Goth house through a full game day agrees with ours
  at every compared tick;
- three more base-game houses agree for their first 1000 ticks;
- recordings in which a player gives orders (scripted clicks and pie
  menu choices in the original) agree with ours given the same orders
  through the player-order API;
- every one of the 49 primitives the install's trees use (`sims census`)
  is ported, for the cases recordings or targeted scenarios reach;
- one expansion-pack lot agrees with recordings of the original.

### Steps

In this order; a step may interleave with the next while a recording is
pending. The integration track's hooks (play mode, player orders, sound
events, and what it asks for after them) come first.

1. **K = 1000 on the Goth house.** Done (#129, 2026-11-15): see the met
   goal below.
2. **A full game day on the Goth house**: midnight and the day's turn,
   both jobs, the carpool and the school bus, mail and bills, sleep.
   Measure the ticks a day takes rather than guess. Record the full
   state sparsely (every tenth update, and every update around events)
   and the random streams every update. Delete the captures and dumps it
   supersedes, and say which.
3. **1000 ticks on three more base-game houses**, picked for code the
   Goths do not reach (for example the Newbies and a family with
   children).
4. **Player orders.** Recordings of the original with the input track's
   scripted clicks and pie-menu choices, matched through
   `Interpreter::pie_menu`, `Interpreter::pick` and `Interpreter::order`.
   These also check the integration track's game.
5. **All 49 primitives**: port the remaining ones as recordings reach
   them, or with targeted scenarios.
6. **One expansion-pack lot**, after steps 1 to 5.

### Progress

- Step 1 is done.
- The integration hooks are merged: play mode (#120), player orders
  (#121), sound events (#122), Idle For Input starting a queued action
  (#131), the pie menu and picking from it (#132), and Go To Routing
  Slot's parameter and global slots (#134), which a player's snack order
  reached.
- Strict mode on the Goth house runs to tick 3602, 08:30 on the first
  day. One tick is two game seconds, so a day is 43,200 ticks (measured
  from the clock globals). The carpool makes the school bus at tick 2703
  (the multi-tile factory is ported). The base layer is kept and the
  pose composed from it, and a person may take itself off the lot in
  its own tick. A multi-tile object may snap where it does not fit
  (0x454e82), and a group is deleted (0x440f37): at tick 3434 the car
  junk's 18 tiles go. At tick 3603 object 354 uses Create New Object
  Instance mode 6 with flag 0x4, which is not ported (the same stop as
  the snack order's). Step 2 continues from there.
- Of the base neighborhood's houses only the Goths' and the Newbies'
  (`House07.iff`) hold a family. Strict mode on the Newbie house runs to
  tick 1802; a route that gives up on a slot it may end near runs the
  Route failure feedback tree (tick 1473). Against the recording of the
  house (updates 0 to 935), every update's random streams, scheduler
  cursor, action serial, globals and objects, and each object's data,
  temps, attributes, frames and person data match, except the words the
  original left from memory it never set (person word 37, locals on half
  a pointer) and global 12 at updates 1 and 3 (the interface's tutorial
  wait). At tick 1803 object 290 needs Create New Object Instance mode 6
  with flag 0x4, which track 3 is porting for the Goths' snack order.

  tick 1472, with no recording to compare yet; at tick 1473 object 105
  gives up a route and needs the route-failure feedback tree (0x45af2a),
  which is not ported.
- The Pleasants (`House06.iff` as the original saved it after moving
  them in, with the Traveling Salesman on the lot) load to the state the
  first tick starts from: the end of the household selection works out
  again who is a visitor, and reading the people raises the action
  serial. Their load equals the capture's but for one person's data word
  50 and last result, which something in the original's load sets and
  which is not found yet. The first frame then stops: person 242's head
  follows person 241, whose animator has never been ticked or drawn, and
  what the original reads for it is not ported.
- The houses of UserData2 to 8 get past the neighborhood: the reload
  makes the records their template files lack. The version-63 houses
  read their arrays whole (Arry 11, 12, 111 and 112). UserData2's
  houses 1 and 9 then stop at their floor and wall maps
  (`World::renumber_surfaces`), and the houses with no family stop at
  the household selection. The templates' `Patch` families are not
  imported (see `docs/decisions.md`).

## Met goal: a headless lot boot that matches the original

Met 2026-11-15 (#129). `sims boot <install> <house> --ticks N` loads a
base-game house, runs its setup scripts and N ticks with no window, and
prints a state hash that is the same on every run and on both CI
platforms. For the Goth house, our state after every one of ticks 1 to
608 equals the fourth capture of the original (full state after every
update), and after every one of ticks 600 to 1000 the fifth, in
everything compared, but for the last place of one head field that
nothing reads (see "Ticks 334 to 366").

How it was reached:

1. **Install detection and content lookup**: `sims install` detects a
   Complete Collection install and builds the object catalog.
2. **Lot loading**: a house file reads into `World`, with a state hash
   and REDB snapshots that survive a round trip bit for bit.
3. **The interpreter**: the load's 22 steps, then the ticks, run the
   original's trees. Unported paths fail loudly and name themselves.
4. **The comparison harness** (`tools/harness/`): the original under Wine
   in a QEMU guest, stopped at every lot update, saving the state at the
   entry of the first tick and after chosen updates, with write
   watchpoints (`--watch`) and memory images. The captures are private.
   Five runs of the Goth house were recorded; the first covered 1000 lot
   updates, tick counts 88445 to 89444. The first does not choose in
   tick 233 and the other four do; we follow those
   (`docs/decisions.md`). With the install's own UserData, the C
   runtime's random stream and all 463 neighborhood records match too.
   The original first draws from the C runtime's stream in tick 6, 1,466
   times, when it makes a person; ours draws the same.
5. **Matching**: the sections below record, load step by load step and
   tick range by tick range, what each needed and what was found.

### Load sequence

What the original does between opening a house file and running the first
tick, in order, and how much of each step `sims boot` does. The met goal
needed every step: a step skipped or run out of order
changes the state the first tick sees.

The order and the addresses come from the old prototype's research on the
1.91 executable. Each step was checked there on its own, against the
original's code run in isolation. Steps 1 to 6 and 8 to 22 are also
checked against captures of the running original (see each step), and
three results of step 7 are (see step 7). Steps 16 to 22 were first read
off a trace of the running original and then from the original's code.

The original does not build a lot for a house. It keeps one lot for as long
as it runs and loads each house into it. So a load starts from whatever
that lot holds: at least the two indicator objects, which every load
creates (in step 13 when the first object comes back onto the lot, or
else in step 14 or 16). `sims boot` has no earlier load, so it starts
from an empty lot (`World::empty`) and creates the two indicators on it.
That starting lot is **unverified**; see the decision log.

| # | Step | Original | Status |
|---|------|----------|--------|
| 1 | Read the house's simulator state into the lot the game is holding, and its model's usage counts | `0x46cfb3`, `0x416568` | Done (`World::read_simulator`, `World::read_house_model`) |
| 2 | Set the globals the running game supplies | `0x46cdc6`, `0x46cfb3`, `0x415eff` | Done (`World::apply_context`) |
| 3 | Resize the held lot, which creates the lot-wide controllers | `0x415e20`, `0x442b9f` | Done for a bare lot that shrinks (`Interpreter::resize`) |
| 4 | Clear the family global | `0x415eff` | Done (`World::clear_family`) |
| 5 | Clear the lot: remove every object on it | `0x44279a`, `0x445f84`, `0x44123f`, `0x440eeb` | Done for objects that are off the lot (`Interpreter::clear`) |
| 6a | Read the house's lot arrays | `0x478599` | Done for house versions 53 to 62 (`World::read_arrays`) |
| 6b | Renumber the saved floor and wall IDs | `0x4a0e3b`, `0x4c5d35` | Done for a house whose maps are empty (`World::renumber_surfaces`) |
| 6c, 7 | Read the house's type table, marking the object types whose scripts changed since the save | `0x43dd76`, `0x43a516` | Done (`Interpreter::read_types`) |
| 6d | Clear the lot again | `0x445f84`, `0x44279a` | Done (step 5) |
| 6e | Construct the house's objects on the cleared lot and read their saved fields | `0x449c5e`, `0x43fe0a` | Done (`Interpreter::read_objects`); a person's graphics only as far as they draw random numbers |
| 8 | Reconcile the lot-wide controllers among the saved objects | `0x43fc02`, `0x442b9f` | Done (`Interpreter::reconcile_controllers`); removing a duplicate only if it is off the lot and no person refers to it |
| 9 | Repair the ground level's object grid | `0x443cb3` | Done (`World::repair_grid`) |
| 9a | Work out the lot's rooms from its walls | `0x479d9a` | Done (`World::rebuild_rooms`); not two crossing diagonal walls in one tile |
| 10 | Give every object its room and ask its function 7 | `0x442659`, `0x433bc5` | Done (`Interpreter::assign_rooms`); a person's room is not told it is occupied |
| 10a | Count the rooms: size, objects, light and score. Twice, with every room marked in between | `0x466c3c`, `0x467bb1`, `0x467e05`, `0x467980`, `0x466ba3`, `0x467094` | Done (`Interpreter::update_rooms`, `World::mark_all_rooms`, `Interpreter::update_pending_rooms`); an indoor room that is dark, has no lights and has people in it fails |
| 11 | Discard pending removals | `0x428679` | Done (`World::discard_removals`) |
| 12 | Select the household | `0x428041`, `0x46d8dc` | Done (`World::select_family`) |
| 13 | Visit every object in registration order | `0x43fc94`, `0x431e2e`, `0x459bf6` | Done (`Interpreter::visit_objects`); a person only if it is doing nothing. On the Goth house all 382 visits run |
| 14 | Create the indicator objects unless the lot has them | `0x445e4a` | Done (`Interpreter::ensure_indicators`) |
| 15 | Finish the household selection: delete people who do not belong, work out the visitors, update the lighting | `0x428041`, `0x42a2b4`, `0x4c4648` | Done for a lot whose people are all members of the household, where it changes nothing (`World::finish_selection`); the lighting is not modeled |
| 16 | Announce the loaded house: with no Sim selected, ask for the indicators again and clear global 3 | `0x428652`, `0x44302e`, `0x445e4a`, `0x46d2c2` | Done (`Interpreter::announce_house`) |
| 17 | Set the funds again | `0x4f1ae9`, `0x46d8dc` | Done (`Interpreter::select_resident`) |
| 18 | Select the family's first member: the Sim's selected mark, the first indicator into the Sim's slot, global 3 | `0x4e97dc`, `0x4e989b`, `0x445449`, `0x44664c` | Done for a lot where nobody is selected and the Sim has no date (`Interpreter::select_resident`) |
| 19 | Go to live mode: every room waits to be counted, the lot's statistics set global 37, then global 19 becomes 0 | `0x4cea5d`, `0x416534`, `0x416085`, `0x4ce44f` | Done (`Interpreter::go_live`); of the statistics only globals 23 and 37 |
| 20 | Count the family's friends | `0x4cfcdb`, `0x427219`, `0x427bd7` | Done (`Interpreter::before_frame`, `World::family_friends`); an empty lot counts nothing (unverified) |
| 21 | Count the rooms that wait | `0x467094`, `0x466c3c` | Done (`Interpreter::before_frame`) |
| 22 | The interface's wait check clears global 12 | `0x4764fe` | Done (`Interpreter::before_frame`) |

`sims boot` runs the steps in order and stops at the first thing that
cannot be done, so it never prints the hash of a state the original could
not be in. Steps 16 to 22 are driven by the original's interface, not by
its loader; what is ported is what they do to the simulation's state. The
first tick (`0x46d345`) comes after them.

#### The steps

**1. Simulator state.** The clock, the 32 or 64 saved globals, the
scheduler cursor, both game random streams and the budget replace the held
lot's. The objects on the lot, its size and the C runtime's random stream
are not in the house file and stay.

**2. Globals.** The simulator zeroes all 64 globals, then overlays the 32 or
64 saved words. It then sets the fields that belong to the running game:
global 15 (mode), 25 (kept from before the load), 29 (always 30), 30 (free
will), 20 (installed expansions) and 33 (hardware performance class). The
speed in global 16 is clamped to 0..1000 and copied to global 18 unless
that is negative. Global 32 (budget mode) is cleared. The saved family in
global 9 stays for now.

**3. Resize.** The house asks for a lot size in global 23, where 0 means
the full 64 tiles. If the held lot has another size, it is resized. The
resize rebuilds the lot's rooms, which writes the lot's indoor area to
global 27, and the object registry answers the rebuild by creating every
lot-wide controller the lot lacks. A lot that already has the size skips
all of this.

A lot-wide controller is an object definition of type 7 or 12 with a
nonzero global word. The original walks the whole catalog, in catalog
order, and makes each missing one through the ordinary object factory:
construct, register at the head of the registration list, stamp with the
lot's date, reset, install Main, run Init, give the object its first turn.
Each new object takes the lowest unused ID. The Complete Collection has 141
such definitions, and our catalog lists them in the order the original
creates them.

These controllers do not last. They run their Init and first turn on a lot
that holds only the two indicators, draw from the global random stream and
write globals and type attributes, and step 5 then removes them. What they
leave behind is what matters to the first tick.

For the Goth house this step is checked against a capture of the original
taken when the resize returns: the lot shrinks from 64 to 52 tiles, and all
141 controllers match the capture field for field, as do the globals, both
game random streams, the registration order and the two controllers
waiting for removal. The gated test
`the_goth_house_resize_creates_the_controllers` pins our state and says
what was compared.

Only this case is ported: a bare lot, shrinking, with every object off the
lot. Growing a lot, a lot with walls or floors, and objects standing on the
lot all fail with an error that says so. The bare lot's rooms are not
built: the lot has no rooms until step 9a works them out. In the trace the
bare lot has two, one for each level.

**4. Family.** Global 9 is cleared. It is set again in step 12.

**5. Clear.** Every object on the held lot is removed: first the two
indicators (`0x445f84`), then whatever heads the registration list until
the list is empty, which takes the controllers step 3 made. The list is
read again after each removal.

Removing an object (`0x44123f`) has two parts. The object leaves the lot
(`0x433125`). Then the registry deletes it (`0x440eeb`): it gives up the
ownership of interface events, every person drops their references to it,
it resets one last time, the objects it holds are removed, and it is
unlinked from the registration list and from its type's instance list. The
last reset (`0x431fbe`) runs the object's Reset tree if it has one, on top
of whatever its thread holds, and then restarts its thread at Main. An
object without a Reset tree just leaves the lot.

The IDs queued for removal stay queued until step 11, though their objects
are gone: the queue holds bare IDs, and whoever drains it skips an ID that
names no object. A type's instance list stays, empty. The clock, the lot's
size, the globals and the random streams do not change, unless a Reset
tree changes them.

For the Goth house this step is checked against a capture of the original
taken when the clear returns. Eighteen of the 141 controllers have a Reset
tree, and all run. Afterwards the lot has no objects, and the tick, the
size, the 64 globals, both game random streams, the two queued IDs, the
instance list and the indicator references all match the capture. The
gated test `the_goth_house_clear_removes_every_object` pins our state and
says what was compared.

Only this case is ported: an object that is off the lot, one tile, not a
person, holding nothing and held by nothing, with no callback for leaving
the lot, and that no person on the lot refers to (see step 8). Anything
else fails with an error that says so. The interface, the objects' graphics and the lot's rooms are not
modeled.

**6. House contents.** The original reads the rest of the house in five
parts, in this order. The order comes from the old prototype's reading of
the original's code between two captured moments of the Goth load: the end
of step 5 and the start of the object read.

*6a. Lot arrays.* Fourteen arrays replace the lot's: IDs 0, 4, 1, 2, 3, 8,
101, 102, 103, 108, 6, 7, 9 and 10, asked for in that order. Arrays 9 and
10 may be absent. A house of version 53 to 62 saves a floor cell as one
byte and a wall cell as 8. The lot holds a floor as a 16-bit ID and a wall
as 14 bytes, and the original widens the saved cells into the lot's own
arrays, for the cells inside the lot's size only. Other house versions
are not ported.

*6b. Floors and walls.* A saved floor or wall names a pattern beyond the
built-in ones by an ID that belongs to the save, and the house lists in
two maps which pattern file each ID means. The original maps the saved IDs
to the IDs of the patterns it has loaded and rewrites the cells inside the
lot's size. Only a house whose maps are empty is ported. There a floor ID
the map should have listed becomes 65535. A wall cell that would change
fails, because the original writes walls through a service that also
updates the neighbouring cells. A map with entries fails too: resolving
them needs the game's loaded patterns.

For the Goth house 6a and 6b are checked against a capture of the
original taken after both. The house's maps are empty and none of its
cells holds an ID the renumbering covers, so 6b changes nothing. Inside
the lot's 52 tiles the capture's terrain heights, floors, walls, object
grid, flags, pool and water equal ours. The capture does not hold arrays
4, 6 and 7, which are **unverified**. It does not hold the cells outside
the 52 tiles either. Those were compared later with the memory a trace
holds at the start of step 10: every such cell it holds equals ours, which
is all of them for the walls, the flags, the ground level's object grid,
the pool and the water. That comparison found one thing wrong. The ground
level's wall cells outside the lot's size are not zero in the original.
They hold a bare lot's cell (no segments, flags 0x0f, both styles 1),
which the held lot had before the load. A lot here has no arrays before
the load, so `World::read_arrays` now puts that cell there.
The gated test `the_goth_house_arrays_are_read_and_nothing_is_renumbered`
pins our arrays and says what was compared.

*6c. Type table.* See step 7, which the original does in the same pass.

*6d. Second clear.* The original clears the lot again. In a load from the
neighborhood the lot is already empty, so nothing happens.

*6e. Objects.* The original first constructs every object the file lists,
as the factory would, and registers each under its saved ID at the end of
the registration list, in the file's order. Then it reads each object's
saved fields into what the constructor made. What the constructor leaves
shows afterwards:

- An object has as many attributes as its definition asks for, never
  fewer than 8. The saved ones fill the front.
- It has a slot for its tile and one for each object it can hold. A
  person has five more. A save with fewer slots does not shrink that.
- A tile of a multi-tile object joins its lead tile's list, in the file's
  order.
- A person's constructor reads the file of its skeleton if the game has
  not read it yet, builds the skeleton, and attaches the person to its
  neighborhood record. If the record gives it no interests, it makes
  default interests and personality. All of that draws from the C
  runtime's random stream. The saved person data then replaces the
  defaults, so only the draws remain.

For the Goth house this step is checked against a capture of the original
taken when the object read returns. All 245 objects match the capture in
every field it holds, except one frame's tree ID on one object, which has
a bit set in the capture that the save does not have; the cause is not
known. The registration order, the globals, both game random streams and
the multi-tile links match too. The three people's constructors draw as
many random numbers as in the original: with the stream's state copied
from the capture's start of the load, it ends in the capture's state. The
gated test `the_goth_house_objects_are_constructed_and_read` pins our
state and says what was and was not compared.

Not ported, and failing with an error that says so: an object with
graphics of its own (definition byte `0xcc` bit 0), a person with no
neighborhood record, a skeleton that two files define, and saved
attributes beyond what the constructor allocates. A person's graphics,
animations and queue bookkeeping are not modeled beyond the random draws.

**7. Changed types.** As it reads each record of the type table, the
original compares the saved Init signature with the linked script's, and
if they are equal the Main signatures too. A type that differs is flagged
on its definition, and step 13 resets its objects. The flag belongs to the
running game: a type the table does not list keeps the flag an earlier
load gave it. The table itself, saved type ID to GUID, is rebuilt from the
definitions that a record named. In the Goth house 77 of 113 types are
flagged. A capture confirms three of them, the Goths' own types: the
original worked out an Init signature of 20 for each, as we do, flagged
them, and never worked out their Main signature. The other 110 are
**unverified**.

**8. Controllers again.** The same reconciliation as in step 3, now among
the saved objects. The original lists the objects whose definition has a
nonzero global word, in registration order, once. Then for each controller
definition, in catalog order, it keeps the first listed instance, removes
each later one at once (as step 5 removes an object), and creates the
controller through the factory if the list has none. A controller created
here runs its Init and first turn on a lot that holds the saved objects
and people.

For the Goth house this step and the next are checked against a capture
of the original taken when the function that does both returns. The house
saved 4 of the 141 controllers and no duplicate, so 137 are created. They
take the lowest unused IDs: the four gaps the house left, then 247 and up.
All 382 objects match the capture field for field, with the one exception
step 6e already had (a frame's tree ID on object 19). So do the
registration order, the globals, both game random streams and the removal
queue. No saved object changes, in the capture or here. The gated test
`the_goth_house_controllers_are_reconciled_and_the_grid_is_checked` pins
our state and says what was and was not compared.

Two of the new controllers ask to be removed, as two did in step 3. The
queue then holds four IDs: 42 and 44 from step 3, and 278 and 280. The
first two now name saved objects, which nobody asked to remove. Step 11
discards the queue before anything acts on it.

Removing a duplicate is ported for an object that is off the lot and that
no person refers to. A person refers to an object when it is using it, or
when its running, last or a queued action targets it or shows its icon.
The original tells every person about a deleted object, and a person that
does not refer to it changes nothing. A person that does must cancel
actions and unwind its thread; that is not ported and fails with an error
that names the person. No capture shows a duplicate being removed, so
that path is **unverified** against the running game.

**9. Grid repair.** The object grid of a level holds, for each tile, the
ID of the first object standing on it. The original walks the ground
level's grid inside the lot's size and clears every cell whose ID is
negative, names no object, or names an object that is not on that tile on
level 1. It does not look at the upper level or at cells outside the
size. Then it walks the objects and logs what it finds wrong with their
containers and tiles; that pass changes nothing and is not ported.

In the Goth house nothing is cleared, here and in the capture. The saved
grid does name eight objects that are elsewhere, all in cells outside the
lot's 52 tiles, which stay as they are. So no capture shows a cell being
cleared: that is **unverified** against the running game.

An earlier version of this table had "restore placement links" in this
step. The original does nothing of the kind here: the function that runs
steps 8 and 9 ends after the grid repair. The links are the grid and each
object's slots (slot 0 names the next object on its tile, the others the
objects it holds), and the object read of step 6e already filled them. A
controller created in step 8 has empty slots. The capture agrees for the
objects whose slots it holds.

**9a. Rooms.** The original works the lot's rooms out from its walls
whenever its architecture may have changed. In a load that is here: after
the objects are read and the controllers reconciled, the loader announces
that the lot changed, and the lot answers by rebuilding its rooms.

A room is a set of tiles that connect without crossing a wall. The tiles
inside the lot's size are scanned column by column, and a tile no room has
taken starts a new room, which spreads to its neighbours depth first. Only
the wall bits count, not the wall styles, so a fence divides rooms. A
diagonal wall cuts its tile into two triangles that may be in two rooms;
the map then marks the tile as split (-5). Upstairs a floor and a hole are
different rooms, and on the ground the edge of a pool divides rooms. The
ground level's rooms are numbered from 0 and the upper level's from 1024.
Each room keeps the list of its tiles in the order it reached them, and a
tile reached again while it waits is listed again. The original counts
with that list, repeats included, so it is kept as it is.

The same function rebuilds the tile flags. Every cell keeps only bits 8
and 0x20. A flood from the lot's corner marks the tiles outside every
enclosure; a wall of a fence style does not stop it. Then each level tile
inside the lot's border is marked as one that can be built on, upstairs
only where something below supports it. A room is outdoors if its first
whole tile is outside every enclosure. A room made only of triangles is
outdoors if it lists exactly one tile.

For the Goth house the result is checked against the trace at the entry of
step 10: 11 rooms where the bare lot had 2, with the same IDs and the same
number of listed tiles; the same listed tiles in the same order for the 8
rooms whose tile memory the trace holds; the same room for every tile of
the upper level in the 32 columns the trace holds; the same number of
split tiles (7); and the same tile flags in every cell of both levels. The
room of every tile of both levels was compared with the trace's memory at
the end of the load and was equal. Whether each room is outdoors was
compared at the entry of step 11, by when the original has worked it out
for all 11. An empty lot of 52 tiles partitions here into two rooms that
list 5,307 tiles each, as the trace's two rooms before the rebuild do.

Not ported, and failing with an error that says so: two diagonal walls
crossing in one tile, a wall that its neighbour does not mirror, an
enclosing diagonal wall on the border of the arrays, and more than 1,024
rooms on a level. Not modeled: two booleans about the upper level, the
roofs, and the redrawing.

**10. Rooms of objects.** When the rooms have been rebuilt, the object
registry runs two passes over the registration list. The first puts an
object whose level is 0 on level 1 and writes the room it stands in into
its data word 29. The second runs each object's function 7 tree, as a
callback on top of the object's thread.

The room of an object is the room of its tile. A tile on the lot's
outermost ring, or off the lot, is no room (-5). On a split tile the
original does not look up the triangle the object is in. It takes the room
of the next tile over, on the object's side of the diagonal.

In the Goth house 62 objects have a function 7 tree: the doors and the
windows. Each asks whether the room on its far side is outdoors and sets
how much light it lets in. Reading that is the one thing these trees need
that earlier steps did not: what a script can read about a room (whether
it is outdoors, its level, its area).

Checked against the trace at the return of step 10. The trace asks
function 7 of all 382 objects in registration order, and touches the
thread of the same 62 objects, in the same order, as have a tree here. No
data word changes in the trace, and none here: every room worked out here
is the room the house saved. All 382 objects, the globals, the random
streams and the arrays are equal afterwards. None of the house's objects
stands on a split tile, so that rule is **unverified** against the running
game; it follows the old prototype, which compared it with the original's
code run alone.

Not modeled: the first pass also tells the room a person stands in that
it is occupied, which lights it.

**10a. Room count.** After step 10 and before step 11 the original
updates its rooms. The update (`0x466c3c`) takes each room that is marked
as not counted, in ID order:

1. It notes the light the room shows.
2. It zeroes the room's counts and clears its mark.
3. It counts each listed tile: two half tiles for a tile, or one for a
   tile with a diagonal wall in an indoor room; how many of them have a
   floor covering; and how many walls can take a covering and how many
   have one.
4. It counts each object standing on the tile, with everything it holds.
   An object is skipped if its data word 34 is not 0 or its room word is
   negative. A person counts as a person. A light source (flag 0x400 of
   data word 40) that stands on the lot adds its data word 51 to the
   room's window light if it is a window, and to its lamp light
   otherwise. Data word 7 is the object's impact on the room's score.
5. It works out the room's light and score. Outdoors the light is the
   daylight, by the time of day in global 4. Indoors it is the lamp light
   plus the window light, per tile of area, the window light scaled by
   the daylight. The score of an indoor room adds up five parts: covered
   walls, covered floor, impact, light and area.
6. If the light the room shows now differs from the one noted, the room
   is announced as changed (signal 0xf1), and the lot answers by marking
   it as not counted again.

Then the lot's indoor area goes to global 27, and the outdoor rooms'
scores are added up into the lot's outdoor score. Last, the levels that
were counted stop waiting, whatever step 6 marked. So a room whose light
changed stays marked, with nothing waiting to count it, until something
marks a room of its level.

In a load the original does this twice. The first update follows step 10
(signal 0xf0). Every room is new and has no light, so every room's light
changes and every room is marked again. Then the end of the house read
announces that the lighting changed (signal 0xdd), which marks every room
and both levels (`0x466ba3`), and counts whatever waits (`0x467094`).
That second count changes no light and leaves every room counted.

The original's FPU runs the score arithmetic with single precision: each
operation's result is rounded to 24 bits. The trace shows it. One room's
score there is the sum of its parts rounded after each addition, three
units in the last place away from the sum rounded once.

For the Goth house everything the update writes was compared with the
trace at the entry of step 11, for all 11 rooms: the twelve counts, the
lights and people, the score and the light bit for bit, the marks, the
waiting levels, the lot's outdoor sum and score, and global 27 (204). All
equal. The trace has no stop between the two updates, so the state after
the first is not compared.

Whether an indoor room is lit because someone is in it (`0x468802`) was
ported later (`World::note_occupant`, `Room::shown_light`): a person
tells its room as it enters and leaves, and an outdoor room and a pool
ignore that. Not modeled, being presentation: the lights the update places in a room, the
lighting of the tiles, and the redrawing. The constant the daylight is
multiplied by is 1.0 in the trace and is taken to be 1.0: **unverified**.
A lot with a pool is **unverified**: this house has none.

**11. Removals.** The queue of objects waiting to be removed is emptied.
The objects stay. In the trace the queue holds four IDs before and none
after, and nothing else it holds changes; the same is true here, and the
state before and after equals the trace's.

**12. Household.** Global 9 takes the family's ID, global 10 the house
number, and the family's money becomes the lot's funds, split across
globals 13 and 14. The family's ID is the ID of its record in the
neighborhood file, not the number inside the record: the Goths are family
5 and their record says 6. The family also becomes the one the game has
selected, which gives the lot its magic history.

Checked against the trace when the funds setter returns: global 9 goes
from 0 to 5, global 13 and the funds from 2,515 to 2,542, the selected
family's ID is 5 and its six magic words are zero, and everything else
compared in step 10 is unchanged and equal. The house saved its own number
in global 10, so the trace does not show that write. A lot with no family
and a family record older than version 9 are **unverified**.

**13. Object visits.** For each object, in registration order:

1. A person takes its persistent fields from the neighborhood record
   (`0x4258a3`).
2. Its position is restored (`0x433e50`): an object on level 0 goes to
   level 1, and the object and everything it holds get their position,
   footprint and room again.
3. A non-person runs its Load function (2). A function whose tree is
   missing is skipped.
4. If its type changed: every person drops references to it (`0x444306`),
   then it resets (`0x431fbe`). An object with a Reset function (3) runs
   it and restarts Main. One without is rebuilt: taken off the lot, its
   storage reset (`0x431be1`), Init and a first turn run, and put back. A
   person has its own reset (`0x45e791`).
5. If its type changed, it runs Init.
6. Its scheduler flags are refreshed from data word 60 and its
   interaction table.
7. A person updates its cached room and reloads its motive curves.

*A person's fields.* The neighborhood file keeps a record for each person
with 256 person data words. The record's ID goes into the person's data
word 31. Then the words the record keeps for the person are copied from
it: 29 words for a record of revision 1 and more for each later revision,
48 at revision 10. They are the person's lasting traits: personality,
interests, skills, age, skin, gender, family, career. The record is the
first of the person's type.

*A person's reset*, for a person that is doing nothing:

1. Person data word 30 is cleared.
2. The appearance is reloaded from the body strings of the person's type:
   person data words 22 and 23 are cleared, and the gender (65), the age
   (58) and the skin tone (60) are read again.
3. The thread restarts at Main. Then the Reset function runs on top of
   it. (An object runs Reset first and restarts afterwards.)
4. Person data words 0, 40 and 59 and data words 34 and 9 are cleared,
   with the motive changes an action left running and person data words
   33, 71, 72 and 73.
5. Each queued action is dropped, and its target is asked function 4.

A person that is using an object, has a running action with a target, is
in the middle of a route, is inside another object or off the lot, or
has data word 73 set fails: the original does more for each, and none was
seen.

*The cached room and the motive curves* are not kept. The cached room
drives the room's lighting, which is not modeled. The curves turn a
motive's level into its share of the mood; nothing reads them until a
person's motives are updated in a tick. `curve_table` says which string
table they come from: the one the person's definition names, or the
global table 501 for an adult and 503 for a child.

Lot time does not advance, and no object's saved Main runs, during any of
this. The next object is read from the list after each visit, so an object
a visit creates, which goes to the head of the list, is not visited.

A visit that cannot be made fails, and `Interpreter::visit_objects` then
stops with the lot as the visit before left it. What the visits needed:

- *Taking an object off its tile and putting it back* (`0x433125`,
  `0x433340`; `placement.rs`). The lot keeps, for each tile, the first
  object on it (the object grid); each object's slot 0 names the next
  object on its tile and its other slots the objects it holds; and data
  words 26 and 14 name the object and slot that link to it. An object
  taken off is asked function 10 first, and the object behind it takes its
  place. An object put on a tile goes in front of whatever is there, and
  is then asked function 9, and function 6 if the walls around it are not
  what its data word 5 says. Whatever it sits in is asked function 9 too,
  and a seat beside the tile sets a flag on everything standing there. A
  multi-tile object moves as a group: all its tiles are asked function
  10, all leave; all are placed, all are asked function 9.
- *The lot's indicators.* When an object comes onto the lot from outside
  and the lot has no indicators, the original's interface creates them
  (`0x44752e`, `0x445e4a`). In a load that is the first rebuilt object
  that stands on the lot: visit 140 in the Goth house. Step 14 then finds
  them made.
- *The frame storage* (`World::frame_storage`). The original keeps a
  thread's frames in one block and never clears it, and a new frame's
  locals start as whatever an earlier frame left there. A Main tree
  restarted by a reset shows it: in the trace, object 12's new Main has
  one local that holds 280, the tree ID of the frame that used to lie
  there. A run of a callback writes an eight-word mark above the top
  frame first. A local that lands on half of a pointer cannot be read
  until it is written.
- *Graphics draws.* An object with graphics of its own draws 32 numbers
  from the C runtime's random stream when the factory makes it. The
  indicators are such objects. The two on the starting lot account for
  the stream's state at the start of the traced load: the initial state
  advanced by 64 draws. So the stream now equals the trace's at every
  stop of steps 3 to 13 without anything copied from the trace.
- *Primitives.* Notify Stack Object out of Idle, Relationship and Old
  Relationship (rows kept on the lot's objects), Create New Object
  Instance (off the lot), Run Tree By Name (pushed on the running
  thread), Set To Next modes 8 and 9 (the objects on a tile, and beside
  one), Refresh modes 1 and 2 (mark a room as not counted), and Generic
  Sims Call 10, 11 (the list of fires) and 43 (the family's magic
  history).

What the trace of the Goth load shows of this step:

- 382 visits. Function 2 is asked of the 379 objects that are not people.
  For 198 objects it is the only function asked. The three people are
  visits 202 to 204, and each is asked functions 3 and 0.
- 184 objects have a changed type. The 3 people and 23 others have a
  Reset tree. The other 158 are rebuilt; 148 of them stand on the lot and
  120 are tiles of a group.
- Two objects are created by a script (385 and 386), besides the two
  indicators (383 and 384). Seven controllers ask to be removed.
- When the step returns, 8 of the 11 rooms wait to be counted again.

Checked against the trace for all 382 visits. The trace has a full state
at the step's entry, in the middle of visit 140 where the indicators are
created, and at the step's return, and for every visit each call of "run
this object's function" (`0x422d46`). Equal: the object and the calls of
every one of the 382 visits, in order (1,781 calls); the whole state in
the middle of visit 140 and when the indicators' creation returns; and
the whole state at the step's return. That last one is all 386 objects
field for field, the three people's data, motives and actions, the 64
globals, the three random streams, the funds, the removal queue, the
registration order, the instance lists, the rooms waiting to be counted
and the lot arrays with the object grids. It is the first check of
everything that is not an object field after visit 140.

What the trace shows of a person's visit: words 46 to 55 of each person's
data change to the record's values, the thread is one new Main frame, the
last result is true, and the Reset and Init trees set seven motives. All
equal here, the motives bit for bit. Everything else the visit writes already holds
the value written in this house, so the trace shows it unchanged. See the
decision log.

One difference seen since step 6e is settled as harmless. Object 19's
second frame has tree ID 0x8118 in the trace and 0x118 here until its
visit, number 206. The visit makes the same calls here as in the trace
and replaces the thread, and afterwards the object is equal in every
field. What sets the bit is still not known.

**14. Indicators.** Two indicator objects are created through the factory
and hidden, unless the lot has them already. In the trace of the Goth
load the function is entered once more after the last visit and creates
nothing: the indicators were made in step 13. The state at its return and
at the return of step 13's function equals ours.

**15. The end of the household selection.** The function that selected
the household in step 12 (`0x428041`) goes on after the visits:

1. It deletes each person that belongs to no family and is on another
   lot's list.
2. In some game modes it creates a tutorial controller object (GUID
   `0xC3249A1D`) if global 26 is not 0.
3. If the family's record asks for it (flag 4), it resets the budget, the
   statistics, the globals and the lot.
4. It works out again which people are visitors (`0x42a2b4`).
5. It deletes each person that lived here before the visits and is now a
   visitor.
6. It updates the lot's lighting (`0x4c4648`).

None of 1 to 5 is ported. `World::finish_selection` fails if any of them
could do something: a person whose family is not the selected one or who
is marked as a visitor, global 26 set, or a family record with flag 4.
Otherwise it changes nothing. The order and the conditions are the old
prototype's, which compared them with the original's code run alone. In
the trace of the Goth load nothing we compare changes between the return
of step 13 and the return of the lighting update. The lighting is not
modeled.

**16. The house is announced.** The loader tells the rest of the game
that a house is loaded (signal `0xf6`). The object registry answers
(`0x44302e`): if no Sim is selected, it asks for the indicators again and
announces that nobody is selected (signal `0xe4` with 0), which the
simulator answers by clearing global 3 (`0x46d2c2`). A house saves the
selected Sim's ID in global 3 but not the Sim's selected mark, so after a
load nobody is selected and global 3 is cleared. In the trace the
indicators exist already, and global 3 goes from the saved 21 to 0.

**17 and 18. Funds and selection.** After a residential house is loaded,
the game's interface (the end of `0x4f1ae9`) sets the lot's funds from the
family's money again, which changes nothing, and then selects a Sim. The
rule: if a Sim is selected and belongs to a family (person data word 61
is not 0), it stays. Otherwise the game selects the family's first member
(`0x4e97dc`): the first object of that member's type. Looking that up
makes the type's instance list. Whether the Sim may be selected depends
on the lot (`0x4e989b`): on a residential lot only a Sim that lives there
(person data word 32 is 0); on lots 21 to 30 any Sim whose word 32 is not
2; on a community lot or lots 40 to 49 a Sim of the selected family or
one whose word 32 is not 2.

Selecting a Sim (`0x445449`) sets the Sim's selected mark, moves the
indicators for it (`0x44664c`), and announces the selection, which writes
the Sim's ID to global 3. Moving the indicators: the first indicator is
taken off the lot, then put into the Sim's slot 1. An object that leaves
the lot is asked function 10, and the registry is told. The registry
takes an indicator that left off the lot once more (`0x447d2b`,
`0x4481eb`), so the indicator is asked function 10 twice. Put into the
slot, it is asked function 9, and so is the Sim.

In the trace: an instance list is made for the type of person 21, global
3 becomes 21, person 21 gets the mark, and indicator 383's data words 2,
3, 14 and 26 become 21, 1, 2 and 21, with the person's room and position.
The callbacks are 383:10, 383:10, 383:9 and 21:9. All equal here. The
second 383:10 was found by running the original's selection code on the
trace's memory in an emulator, which showed its caller.

Ported for a lot where nobody is selected yet. Deselecting a Sim fails,
and so does a Sim with a date (the second indicator marks the date) and a
Sim whose slot 1 holds something. Three things belong to the interface
and are taken as the trace shows them, **unverified** otherwise: every
family member is on the interface's list of portraits; a process-wide
switch that lets any Sim be selected is off; and a slot of a person takes
an indicator.

**19. Live mode.** The mode changes (`0x4cea5d`). The lot's part
(`0x416534`) marks every room as not counted and works out the lot's
statistics (`0x416085`). Then the interface writes the mode, 0, to global
19. The statistics pass sets global 37 from the categories of the lot's
objects (data word 59), one bit for each kind of object, and global 23
to the lot's size. It also writes global 22, the value of the lot's
objects and architecture, and two flags of the family's record; those
are not ported and stay as they are, which is **unverified**: in the
trace they do not change, but they already hold what the house saved. In
the trace the three rooms that were counted are marked, global 37 goes
from 0 to 1, and global 19 from 3 to 0.

**20. Friends.** The interface's family panel shows the selected
family's friend count, which works the count out again: every family's
count is out of date from the neighborhood's read. In the capture the
Goth family's count is 0 and up to date, and every other family's is
out of date, as ours are.

**21. Rooms.** Before it draws a frame the game counts the rooms that
wait (`0x467094`), as in step 10a. All 11 rooms are counted. The five
outdoor rooms end counted. The six indoor rooms end marked again: their
light changed. At step 10a they were fully lit through their windows;
step 13 reset the windows, which cleared the light each lets in, so now
they are dark or lit by lamps only. This is what the earlier version of
this file could not explain. In the trace every count, light and score of
all 11 rooms equals ours, and so do the marks.

**22. Wait check.** The interface's wait check (`0x4764fe`) writes what
it found to global 12: the control a tutorial is waiting for, or 0. With
no interface it is 0. In the trace global 12 goes from 2009 to 0.

Then the trace stops at the entry of the first tick (`0x46d345`). No
object's thread runs, the tick count stays at 88,444, and no random
stream moves in any of steps 14 to 22. Our state after step 22 equals the
trace's there: all 386 objects, the people, the globals, the three random
streams, the funds, the removal queue, the instance lists, the rooms and
the lot arrays.

### The first tick

`crates/sims-vm/src/tick.rs` runs ticks. Each piece below was read from
the original's code. "Matches" means compared with a capture of the
original and found equal.

**One lot update** (`0x46d345`, `Interpreter::lot_update`):

1. The game's random stream is swapped for the lot's (`0x46f447`,
   `0x46f44d`), the scheduler makes a pass over the objects (`0x46d553`),
   and the stream is swapped back.
2. If the pass was a full one, global 6 (seconds) goes up by 2 and the
   tick count by 1. At 60 seconds global 5 (minutes) goes up and signal
   0xdb is sent. At 60 minutes the hour turns (global 0); at 24 hours
   the day turns, which is not ported and fails loudly.
3. The registry's pass at the end of the update (`0x4412d1`,
   `Interpreter::end_of_update`). It is not only the removals, as this
   file said before. In order: the dialogs (a new one is shown, and a
   modal one is answered there and then; a finished one is destroyed);
   the game's special panels, which no ported primitive asks for; after a
   full pass, the queued removals, oldest first (`Interpreter::remove_now`
   for each, `0x44123f`); and a check that a Sim of the household is
   selected (selecting another, `0x445501`, is not ported and fails).
4. Signal 0xdb goes to the game's eight subscribers. Only the interface
   takes it (a sound on the hour), so it changes nothing here.
5. If the hour turned and the time of day changes with it (`0x46d525`),
   global 4 takes the new one and the lot marks every room for counting
   (signal 0xdd). See "Ticks 878 to 900".

**One scheduler pass** (`0x46d553`, `Interpreter::scheduler_pass`). The
pass is partial if global 17 is not 0, or globals 19 and 10 both are not.
It goes by object ID from the cursor (or 1) to the highest ID at the
start. An object without flag 1 is skipped, and in a partial pass so is
one without flag 2. A countdown above 0 is counted down by 1; otherwise
the object is ticked with the tick count. A tick that returns false
stops the pass with the cursor kept; no ported primitive makes one
return false, so that fails loudly.

**An object's tick** (`0x4521f7`, `Interpreter::tick_object`): its thread
runs until it yields, and data word 25 counts down. A thread that returns
from its outermost frame (error 0x3e9) or runs past the instruction limit
(error 0x44d) fails loudly; the original's error handler is not ported.

**A person's tick** (`0x45cc8c`, `Interpreter::tick_person`): the object's
tick; every 60 ticks a decay of the motives (`0x420fb6`, in single
precision, with the mood); the room the person was last seen in, and
motive 13 from that room's score; person word 27 counted up; the motive
changes of the running action (`0x45cdb3`). Every tenth tick an awake
household member is counted in the house's usage counts (`0x4165e3`,
`HouseUsage::sample`): the day's total, and the day's routing count if
the person is in the middle of a route. The counts come from the house
file's model (HOUS 1, `0x416568`). This file said before that the person
asks the lot something there; it does not. The capture does not hold the
counts, so they are **unverified**; nothing ported reads them.

**Dialog** (primitive 36, `primitives/dialog.rs`, `0x451552`): the first
time it makes a record and yields. The registry shows the record at the
end of the update and the player's answer goes into it. The next time the
primitive reads the answer: the first button is the true branch, the
second and third the false one. The player is `sims_vm::Player`; `sims
boot` answers a one-button dialog with its button and a dialog with a
choice from `--answers`, and fails if none is left.

**Compared with the original.** A second run of the original on the same
install and house (private) holds the state at the entry of its second
lot update, which is the state after its first tick, and the states after
ticks 2 to 5. Our state after tick 1 equals it in: the tick count, the
lot's and the game's random streams, the scheduler cursor, the funds, all
64 globals, the removal queue, the registration order, the set of objects
(the nine removed are the same), and for each of the 377 objects its
GUID, data words, temps, attributes, scheduler word, last result, frames,
relationships, slot count, position and two marks; for the three people
their person data, motives and previous motives bit for bit, actions,
queues and objects in use. The C runtime's random stream is 52 draws
further on in that run, before and after every tick it holds; nothing in
tick 1 draws from it here. The likely cause is that run's game before its
load, but that is unverified. Not compared: rooms, lot arrays, dialog
records and the persons' tick state, which that capture does not hold in a
form we read.

**The neighborhood's people** (`crates/sims-sim/src/neighbors.rs`). A
*record* is the original's 628-byte entry for one person of the
neighborhood: an ID, the GUID of the person's type, 256 words of person
data, and relationship rows. The game reads them from the neighborhood
file (NBRS, `0x42feb2`) and rebuilds the family word of each when the
neighborhood reloads (`0x424806`), before any lot loads; the inventories
come from the NGBH chunk (`0x4254f6`). `World::read_neighbors` does both.
A script names a record by its place in the list plus one (`0x4256f4`).
What reads and writes them: Set To Next modes 5 and 7 (`0x425b15`),
expression scope 32 (a record's person data, `0x4537e3`), Relationship
with flag 2 or between two people (`0x44d17c`), and Manage Inventory
operation 3 (`0x4552f8`). After tick 1 all 463 records and the
inventories equal the capture's, when ours reads the same neighborhood
file as the capture's game; the install's file differs from that one in
five records (see `docs/decisions.md`). One registry byte that a
relationship write sets is 1 in the capture and 0 here; what sets it
during the original's load is not known.

**Tick 2.** Object 16, a person, runs a tree of the people's semi-global
file that goes over the neighborhood's people and their relationships.
It then waits for input (Idle For Input, `0x45db94`), measures distances
(Get Distance To, `0x451f27`) and reaches Animate Sim (`0x58950b`). That
primitive starts a clip in the person's animator and asks it, each run,
for its events and whether the clip is done. The animator's tick
(`0x58dc72`) runs its clock, moves its speed, places a standing person
and advances the clip; `crates/sims-sim/src/animator.rs` says what of it
is kept and what is left out as presentation. The capture shows object
16 at the same node, with the same arguments and locals, after ticks 2
to 5: the clip does not end in that time, and here it is still playing
too. After tick 1 and where tick 2 stops, the three people's animators
equal the capture's in every field its memory pages hold.

Later in tick 2, object 364 makes object 262 again and puts it near
another object with Find Location For (`0x44b3c8`,
`primitives/find_location.rs`). That searches the lot in a square spiral
for a free tile in the same room, then anywhere (`0x4347f0`), tests the
tile with the object's place test (`0x433311`, through `0x4329c1`,
`0x476f5a`, `0x4337f6` and `0x476dbc`; `placement/find.rs`), and puts
the object there. It draws no random numbers. Then object 15 asks a
two-button dialog, answered with the second button (3), as the capture's
events show. The state after tick 2 equals the capture's, object 15's
frames included, so the dialog comes in tick 2; this file said ticks 3
to 5 before.

### Ticks 2 to 5

In tick 3, people 18 and 21 start a 999 ms clip of 30 frames, and 35
objects are removed. The script names the clip "-Rstand"; the
animation library holds only "-rstand". The library's lookup
(`0x5a27f2`) compares bytes exactly, yet the capture shows the original
playing "-rstand", so somewhere the case is ignored. Where is not found.
`Animations::clip` now falls back to the one name that differs only in
case, and fails if there are two. Ticks 4 and 5 count down and run the
same four objects.

**Compared with the original.** After each of ticks 2 to 5 our state
equals the capture's at the return of the same lot update, in every
field listed for tick 1, floats bit for bit; also the people's animators
(every field we keep that the capture's memory pages hold) and, read
from the neighborhood file the capture's game read, all 463 records, the
inventories and the registry byte that differed after tick 1. Not equal:
the C runtime's random stream, still 52 draws further on in the capture;
neither side drew from it in these ticks. Read from the install's
neighborhood file, five records and one inventory differ, as after tick
1, because the two files differ there.

### Ticks 6 to 13

In tick 6 object 382 makes a new person of type 0x44A86A6C where the
pedestrian portal (object 6) stands: Create New Object Instance mode 8
with flag 0x20 (`0x44e0c8`). The person path of the factory (`0x440b2d`
from `0x440d63`, `Interpreter::create_person`) gives it the lowest unused
ID, 259; builds its skeleton, drawing from the C runtime's stream;
attaches it to its neighborhood record; runs its construction reset
(`0x4598e8`), which runs its Init twice and its first update once; and
sets its price and ready flag. Mode 8 then gives it the record's
interests and personality (`0x425930`), the stack object's facing whole,
and places it with the place test of a person (`0x4337f6`, `0x433975`).
From tick 7 the person runs its own scripts and sleeps.

**Compared with the original.** After each of ticks 6 to 13 our state
equals the 1000-tick capture's in every field listed for tick 1, the C
runtime's stream included, and the four people's animators.

### Ticks 14 to 33

In tick 14 person 259 runs a tree of object 51 with Run Tree By Name,
which starts its use of that object (`0x45e59b`, `Machine::count_use`):
an entry in its list of objects in use and one more in object 51's data
word 62. In that tree it reaches Go To Routing Slot (primitive 45,
`0x45abce`, `primitives/go_to_slot.rs`), which is now ported for that
route:

- The route context (`0x48163e`) is pushed on the person's contexts
  (person `+0xaf0`, `sims_sim::route`).
- The places the person could stand are found (`0x48235a`,
  `0x483414`, `routing/slot.rs`).
- The route is planned (`0x45c38e`, `0x4823fa`, `routing/planner.rs`):
  the obstacles of the room's objects, the room's own rectangles
  (`0x4818af`, `routing/rooms.rs`), and the search over free rectangles
  (`0x469b26` to `0x46a8e1`, `routing/regions.rs`, `routing/search.rs`).
  The path has 11 points.
- The animator starts the route (`0x589c85`) and the thread waits while
  it is under way.

The animator then follows the path (state 9, `0x58e4e0`), finds that
the first point ahead is half a turn behind it (`0x58eb29`) and turns on
the spot with the posture's half-turn clip (states 3 and 4, `0x58df01`,
`0x58e045`). It is still turning after tick 33.

In tick 24 person 18's clip ends. The end of a clip in Animate Sim is
now ported (`0x589c25`): the animator stops playing, takes its state
from the posture (`0x592904`) and brings its base layer up to date
(`0x590430`). Starting the next clip clears the finished one from the
foreground slot at once (`0x586b63`).

**Compared with the original.** After each of ticks 14 to 33 our state
equals the 1000-tick capture's in every field listed for tick 1, the C
runtime's stream included, the four people's animators with their route
fields, and each person's number of route contexts. The context, the
planner's inputs, both of its searches and the path are not on the
capture's pages. They were compared with the original's own code run in
an emulator on a memory dump of the original at the entry of update 14,
and are equal. Person 259's word 37 is not compared: the original copies
it from memory it never wrote (`0x45c418`).

### Ticks 34 to 232

**What wrote person 259's direction.** A run of the original with a
hardware write watchpoint on the direction of person 259's animator
(`+0x88`) stopped outside the lot update, between updates 33 and 34, at
`0x584895`. The call chain was `0x45e36b` (the person's draw, virtual
function `0x20`), `0x4ab67d` and `0x4a945d` (the house view drawing its
objects), `0x4a8deb` (the house view's draw), `0x4d1bbb` and `0x4d0dea`
(`DDDSimsView::Draw()`), `0x5dcffe` (the main loop). It is the animator's
first draw (`0x590c05`): byte `+0xdb`, set when the animator is made, is
cleared, the position is unset (`0x589361`), and the animator is placed
again on its person's tile, facing the person's direction word, which is
0 (`0x592132`). The destination follows. This is presentation, but it
writes state the simulation reads, so `sims_vm::Interpreter::draw_frame`
models it (`frame.rs`).

A second run watched the house view's draw guard (`0x4d1b60` sets it,
`0x4d1be7` clears it). The view drew before the first update, after
updates 1, 2 and 3 (many times while a dialog waited), and after update
33; not after updates 4 to 32 or 34 to 45. The first frame drew people
16, 18 and 21, whose bytes `+0xd8` to `+0xdb` were already 0 before
update 1. Person 259, made in tick 6, was first drawn after update 33.
When the game draws is up to its main loop, not the simulation, so the
frame schedule is an input (`sims boot --frames`). Three later runs
drew before update 1, after updates 1, 2 and 3 (how many times differs
from run to run), after update 33 and after every 30th update from
there. The default follows them: `0,1,2,3,33+30`. Each frame counts
the rooms that wait (`0x4a8895`), so frames at other times give other
room values: the frame after tick 2 counts rooms 1 to 4, which a script
marked in that tick. Without the frame after 33, ticks 34 on differ.

**What wrote registry byte 0x12.** The watchpoint on the byte stopped at
`0x43fc8d`, in the end of the object read (`0x43fc02`), during the load,
going from 0 to 1 (`World::finish_object_read`). It stopped again at
`0x442521`, in the end of a full lot update (`0x4424cb`), going to 0 in
update 36: the update clears the byte when the tick count is a multiple
of 40. Both are simulation and are modeled.

**The walk.** After the first draw, person 259 turns again from
direction 0 and ends the turn in tick 47. It then heads for the point
ahead and walks (`0x58eb29` to `0x58f089`): the walk clip repeats in the
foreground (end mode 1, `0x597dca`); each tick moves the animator by the
clip's distance times 16/3, divided by the clip's duration at the
animator's speed counted in ticks of 42 milliseconds; the step is moved onto the path where that is nearer
(`0x48313e`, `0x45ed5f`), the person is put on the step's tile with the
place test, and its room is noted (`0x45e43d`). The path's segments are
followed (`0x58e57d`).

**Timed events.** From tick 46, clips reach their timed events
(`0x5912fd`, `clip_events.rs`): `xevt`, `interruptible`, the hands and
the censor words. Sounds are not modeled. `anchor` feeds only code
behind byte `0x6f6d70`, which nothing writes and which is 0 in a memory
capture of the running game, so it is not kept. `dress` and `undress`
fail. Animate Sim's clip 0 stops the foreground clip (`0x5897aa`).

**Compared with the original.** After each of ticks 34 to 100, and after
ticks 110, 120 and every tenth to 230, our state equals the capture's in
every field listed for tick 1, the four people's animators (walking flag
included) and each person's number of route contexts. The
`updates.jsonl` line is equal after every tick to 232.

**Not verified.** The state after ticks 101 to 109, 111 to 119 and so on
is not in the capture. That the draw's other writes (the pose, the draw
bounds, the screen rectangle, the skin's censor update) change nothing
the simulation reads. That nothing reads a layer's anchor fields but
the pose. Why the original drew no frame after updates 4 to 32 is not
known; it depends on the main loop's timing under the harness.

### Tick 233

**Find Best Interaction** (primitive 3, `0x45dd39`, `autonomy.rs`). Person
21 scores the interaction table entries of the lot's objects against its
motives, with its score curves, and picks one: six candidates, sorted with
the C runtime's `qsort`, then one draw from the lot's random stream. It
chooses menu entry 6 of object 132.

**Gosub Found Action** (primitive 30, `0x45a859`, `found_action.rs`). The
entry's action tree is pushed on the person's thread, the person starts
using the object, and the interaction becomes its running action
(`0x45e6cd`), with the first unique action number (`0x6e4904`, kept as
`World::action_serial`; snapshot format 15).

**Go To Relative Position** (primitive 27, `0x45c634`). The slot it routes
to is built from the operands (`relative_slot`), and the route's steps are
those of Go To Routing Slot. The object is in another room, so the route
goes through a door (step 2, `0x45afd0`).

**Compared with the original.** The second capture's watchpoints saw six
candidates, the choice of menu 6 (`0x45e28b`) and the action number go
from 0 to 1 (`0x41f15e`) in update 233. Ours does the same, and the
state after tick 233 equals the second, third and fourth captures'.
Find Best Interaction's steps were also compared with the original's
code run on a memory image at the entry of update 233.

**The first capture.** Its original did not choose in tick 233. Its one
draw from the lot's stream in ticks 233 and 234 comes in tick 234, and
after tick 240 person 21 runs tree `0x208a` and has no running action.
Its state after tick 230 equals ours as far as compared, and the
original's code run in an emulator on its memory from update 47 chooses
in 233. So something outside the lot updates, after update 47, changed
the outcome in that run only. What it was is not known (see
`docs/decisions.md`). We follow the three runs that agree.

### Ticks 233 to 261

**Routing between rooms** (`routing/portals.rs`). A door (an object of
type 8 with a tree for function 15) is a portal from its room to the
room of its other side. Each portal holds a vector of costs (object
`+0x1ac`, saved with the house), one column per number handed out from a
pool that people and portals share (object data word 6, `0x46381a`). A
person's column holds the cost of reaching the target through each door
of its room (`0x463c83`); a portal's column holds its shortest distance
to every other portal (`0x46390f`). The route's step 2 (`0x45afd0`)
picks the cheapest door, notes the try (`0x463e43`; a second try marks
it failed, as does a door tree that returns false, `0x463e09`), and
pushes the door's tree on the person's thread. That tree walks the
person to the door and through it with Go To Routing Slot.

**Slots searched by distance** (`0x483598`). A slot with flags in its low
byte gives rows of places in each flagged direction. Each place is
classified (`0x481439`): outside the lot, another room, closed off by a
wall or an object (`0x4359ea`), on uneven ground, or free. The last
step of a route to a slot follows the slot's facing (`0x45c516` to
`0x45c5fb`). The planner now plans in a room indoors or upstairs (a
free rectangle costs 0 there), and searches without the width limit in
a room that is small or crowded.

**The end of a path** (`0x58e740`). For a person whose object data word
9 is set, the animator walks on toward the end point until it is near,
then stands. Standing with the route's end noted clears the walking
flag (`0x58f117`). The route then ends (`0x589d47`, `0x45b6ea`): the
person is put on its tile, turns along the path's last segment, and the
context is removed.

**Rooms and people.** A person whose room changes tells the old room it
left and the new room it is there (`0x468802`). A dark room with no
lamp then shows the light of a room with one (0.8, `0x4688bf`), so its
light changed and it waits to be counted again. In a load, each person's
visit notes its room too (`0x45e43d` at `0x459c22`). Each frame counts
the rooms that wait before it draws anything (`0x4a8895` at
`0x4a8b03`). Without that count, motive 13 of a person differed from
the capture after tick 260. Snapshot format 16 keeps the room's mark
and the animator's last step.

**Compared with the original.** After every one of ticks 1 to 261, our
state equals the fourth capture's in every field listed in
`the_goth_house_first_tick_matches_the_original`, the people's
animators, route contexts and paths, the doors' portal costs, and the
`updates.jsonl` line. Only the fourth capture holds the portal costs.
The second capture is equal after every tick it holds (to 240). The third
is equal but for global 12 after ticks 1 and 2 (17 there, 0 here and in
the second and fourth) and one local of object 18's seventh frame after
ticks 210 and 220 (16104 there, 0 here and in the fourth). Both differ
between the original's own runs.

**Not verified.** That the route context's word `+0x74`, which the
original leaves as the heap held it, is never read. That nothing
outside the compared fields depends on the frame schedule within
updates 1 and 2.

**Stops.** Tick 262 stopped here, at the end of a path with object
data word 9 clear (`0x58e8be`); see "Ticks 262 to 333".

### Ticks 262 to 333

**The end of a path with data word 9 clear** (`0x58e8be` to
`0x58eb03`). The person walks on and slows down, and a timer (animator
`+0x1c0`) says when it stops. With no timer set, the phase of the walk
clip picks the next footfall, and with it the stride that ends there.
Within that stride of the end, the timer is set to the time left to
the footfall, and the clip's weight fades to the share of the stride
left (`0x597a3e`; a layer's fade, `0x597e3c`, is now ported). A clip
that may not be interrupted and has not finished only marks the
motion as near its end. When the timer runs out, the person is put on
the end point, faces along the path's last segment, and stands. The
turn on the spot now takes every turn clip, not only the half turn
(`0x58f4ee` to `0x58f688`).

**The walk's strides.** The person's constructor (`0x58831d`) poses
the skeleton with the walk clip at each of the clip's two footfalls
(its `anchor` events) and keeps how far apart the feet are along one
axis (animator `+0x17c` and `+0x178`). We work them out the same way
when they are needed (`sims_sim::pose`, `Interpreter::strides`): the
skeleton's bones and the clip's samples are now read, and the pose is
taken in single precision, as the old prototype's 24-bit variants do.
Our strides equal the ones the original stored, bit for bit, for all
three people. Getting there needed a fix to the sample decoder: the
original decodes a sample's changes through a table it builds at
`0x5975d4`, each step rounded to single precision. Ours now builds the
same table; the old formula, in double precision, differed in the last
bits of most entries.

**Animate Sim names a clip from an object's own tables** (source 0,
`0x59dd6d`): string table 129, 130 for a child, and 131 or 132 when the
clip is for an interaction with a person of the other age.

**Look Towards, mode 0** (primitive 22, `0x45a98e`): the head follows
the stack object, through person data words 41 to 45 (`0x58a035`).
Modes 1 to 3 turn the person on the spot and are not ported.

**Snapshot format 17** keeps the motion's timer and each layer's fade.
The pins taken under format 16 still hold with them left out.

**Compared with the original.** After every one of ticks 1 to 333, our
state equals the fourth capture's in the same fields as before, now
with the timer and the fade, and the `updates.jsonl` line.

**Not verified.** The strides are worked out with the layer's time since
its last pose taken as 0 and the clip taken as looping. Neither is read
from the original; the results equal what it stored.

**Stops.** Tick 334: person 16's head follows its target (`0x5907c5`),
which needs the places of its bones in the current pose (`0x5904d2`).

### Ticks 334 to 366

**The head follows its target** (`0x590768`, `0x5907c5`). After Look
Towards, each animator tick turns a person's head a little towards
its target. Two angles are measured, seen from above, in eighths of a
turn: the head's own heading, from the top of the head (the HEAD
bone's "Top" anchor) to a point in front of it ("HeadFake"), and the
target's heading, from the top of the head to the target. The turn
moves by at most 0.2 towards their difference, and stops at 1.5 either
way; then the script's word 42 changes. With the follow off, the turn
shrinks by a fifth each tick. The angles are measured on the current
pose (`0x5904d2`): the pose the foreground clip gives at its phase,
with the head turned, placed in the lot by the person's direction and
position (`0x592132`). The person stands at the height of the ground
under it (`0x478dfe`): on a sloped tile, the height comes from the
tile's four corners, on one of the two triangles its diagonal makes.
`sims_sim::head` holds the arithmetic, and the animator now keeps the
head's fields (animator `+0x210` to `+0x224`).

**A walk that starts with a clip** (animator state 6, `0x58e125`).
Person 18 plays clip `0x13`, which carries it one clip's distance
straight ahead. The animator moves it along the clip until the clip's
phase reaches 0.95.

**Joining an interaction** (`0x41ee99`). When Find Best Interaction
looks at a person whose running interaction may be joined, it also
scores the joining entry of that interaction's object. The entry's
check tree runs with a second argument of 1. An entry whose join
interaction is missing is left out, as in the original.

**Run Tree By Name, mode 1** (`0x44ba26`). The tree runs to its end on
the stack object's own thread, with that object's temps, and its
result picks the branch. Mode 0, on the running object's own thread,
is not ported.

**When a place is refused, why** (`0x48158c`). The place test now tells
its refusals apart. A wall in the way answers 3. Another object in the
way answers 5 when it is a person and 1 otherwise, and the route keeps
which object it was (`+0x8c`). With no candidates, the planner gives up
(`0x482462`), and the route to a slot then clears its candidates
(`0x481898`).

**Snapshot format 18** keeps the head's fields. The pins taken under
format 17 still hold with them left out.

**Compared with the original.** After every one of ticks 334 to 366,
our state equals the fourth capture's in the same fields as before, now
with the head's fields, with one exception. Person 16's target height
(animator `+0x220`) is one unit in the last place lower in ours after
ticks 335, 336, 338 to 340, 342, 343 and 353. Why is **unknown**: the
height of the ground was checked exact, so the difference is likely in
the pose, whose bones the captures do not hold. Nothing in the
simulation reads that field; the headings use the other two.

**Not verified.** The sine, cosine and arc tangent come from libm in
double precision; the original's FPU instructions may round
differently at some inputs. Run Tree By Name mode 1 returns false when
the object's thread is in an error state; no object here is.

**Stops.** Tick 367: person 18's route to a slot finds no way, because
person 21 stands in it. Step 6 of the route tries again and asks the
person in the way to move (`0x45b066`, `0x45ba28`); it is not ported.
(Ported since; see "Ticks 367 to 367".)

### Ticks 367 to 367

**A route tries again** (route step 6, `0x45b066`). When the planner
finds no way, the route counts its tries in the context (`+0x58`). On
the first try it compares the person's place with the one it noted
(`+0x60`): a person that moved more than 8 units starts over from the
places to stand. On the second, it looks for a person in the way and
asks it to move. The third starts over once more; the fourth gives up
(step 16: the result becomes 1 and the route ends as failed).

**Asking a person in the way to move** (`0x45ba28`, `0x45bf2e`). The
other people near the route are the candidates. The route is planned
once leaving each candidate out; when none gives a way, it is planned
leaving nobody out, and the path is sampled once per unit of its longer
side. The first candidate whose footprint holds a sampled point is
asked, unless it is already asked to move or its own route waits on
someone. A marker object (`0x7c4`) is made where the asked person
stands, with a move slot of its own (`world.routing.slots`): to either
side of the way the asking person goes, 16 to 80 units away. The slot
search for that slot adds a random part to each score (slot flag
`0x2000`, `0x483b02`), one draw from the lot's stream per place in
range. The asked person is given an action on the marker (its
interaction 3, at the asking person's running priority), queued behind
every action of at least that priority (`0x45d20b`); the action's
number goes into the context (`+0x90`, snapshot format 19).

**Waiting for the person asked** (route step 14, `0x45b475`). The
asking person plays "route ask to move animation" with the asked
person as stack object, then "route waiting animation" for up to 100
ticks, then tries again. When a route ends, other routes waiting on
that person forget it, and the action it asked for is taken out of the
asked person's queue (`0x45b89b`, `0x45d5f8`).

**Look Towards turns on the spot** (modes 2 and 3, `0x45a9c1`). The
person's animator runs a route from its own place to its own place and
then to the stack object's place, or the place mirrored through its
own in mode 3. Mode 1, towards the camera, is not ported.

**One pass per place in the slot search** (`0x483a5d`). The search's
loop over the standing place, the eight seats and the standing place
again scores each place once, at the first or the last index, whichever
weight is larger. We scored a refused place twice, which only showed
once each score drew a random number.

**Compared with the original.** After tick 367 our state equals the
fourth capture's in every field compared, including the lot's stream
(72 draws), the marker, person 21's queue and the action serial, with
the same head-height exception (now also after tick 367).

**Not verified.** Only the paths the Goth house takes are exercised:
the first try's place comparison, the second try asking, and step 14's
first push. Steps 14's waiting tree, the third and fourth tries, giving
up and the release at the end of a route are ported from the code but
not reached by tick 367; the capture shows them later (the waiting
tree from update 409, the route's end and the queue cleared at 489).

**Stops.** Tick 368: person 21's tree `0x1012` changes its suit (Change
Suit/Accessory, opcode 6), which is not ported. (Ported since; see
"Ticks 368 to 408".)

### Ticks 368 to 408

**Change Suit/Accessory** (opcode 6, `0x45d962`). In tick 368 person 21
takes a book from the bookcase, whose tree puts on the accessory named
by string 0 of its table 304, "suit-book101", lowered in ASCII. Putting
an accessory on (`0x58d88f`) takes off any of that name and adds one at
the end with an empty binding; taking it off (`0x58d919`) drops every
exact match. The suit form (operand 1 = 1) sets the suit (person word
8) and a variant (word 21), drawn from the active stream for suits from
8 on; the Goth house does not reach it.

**Compared with the original.** After every one of ticks 368 to 408 our
state equals the fourth capture's in every field compared, with the
same head-height exception. The capture shows person 21 wearing one
accessory from update 368 on; the entry itself is on a page the capture
lacks.

**Not verified.** The original keeps an accessory's name only when its
suit is found and binds to the skeleton (`0x583559`); otherwise the
entry stays with an empty name. Ours refuses, loudly, a name that no
decoded avatar file defines (`People::suits`) and takes any other to
bind; that this matches the original's registry is not verified. The suit
form and taking an accessory off are ported from the code but not
reached.

**Differs.** Tick 409, until fixed in "Ticks 409 to 416".

### Ticks 409 to 416

**The person vector.** In tick 409 person 18's route plays "route
waiting animation" (step 14). Its tree looks round the people in its
room (Set To Next mode 1, then a draw for each), and the original looked
at person 259 where ours looked at person 16. Set To Next walks the
original's person vector (`0x44efb4`), not the list of objects, and
person 259, made at run time, comes after the people the load read,
where it heads the object list. The world now keeps that vector
(`World::people`, snapshot format 20): the load's people in
registration order, each person made since at the end, and a removed
person out. Modes 1 and 12 walk it.

**Compared with the original.** After every one of ticks 409 to 416 our
state equals the fourth capture's in every field compared, with the
same head exception. The two orders draw the same number of times, so
the random streams agreed before the fix too; only the person looked at
differed.

**Not verified.** That a person made at run time joins the end of the
vector is read from the draws in tick 409, not from the code that adds
it, which was not found; nor is when in the making it joins. No capture
page holds the vector.

**Stops.** Tick 417: person 21 runs Find Best Object for Function
(opcode 14), which is not ported.

### Tick 417

**Find Best Object for Function** (opcode 14, `0x44f2bf`,
`find_best_object.rs`). Person 21's bookcase tree (0x1014) asks for the
best object for function 8. The primitive walks the lot's objects in
registration order and scores each that has an action for the
function's callback: its score (an attribute, or 50) less the
"functional distance attenuation" (3) times its distance, and at least
1. Room flags, a check tree per entry, and an origin other than the
running object narrow or change the walk. It picks object 174, the
stack object the fourth capture shows.

**Not verified.** The primitive against a capture of its own; tick 417
has not run to its end yet.

**Animate Sim on the carry slot** (flag 0x10 alone, `0x589683`;
`animate.rs`). The same tree then plays a clip on person 21's carry
slot. The clip starts there and repeats at priority 10, the carry mode
becomes 6 so the animator's tick leaves the slot alone, and the tree
takes the true branch at once to node 7, as the capture does. Clip 0
clears the slot. The animator keeps the carry layer (snapshot format
21) and updates it with the foreground layer in the skin's list order.

**Run Functional Tree** (opcode 20, `0x44f681`,
`run_functional_tree.rs`). Node 7 of the same tree runs object 174's
action for a function: the function names a callback in the stack
object's function table, and its action tree is pushed on the running
thread as a call, the way Run Tree By Name's mode 2 pushes, with the
running object's temps as its arguments and a person's use of the
object noted. Unlike Run Tree By Name, the frame keeps no state, so the
instruction takes the branch of the tree's result. Object 174's tree
8215 runs trees 4105 and 4112 of object 52, and the tick ends with the
same frames and object uses as the capture.

**K = 417.** Our state after tick 417 equals the fourth capture in
everything compared, but for the head field of "Ticks 334 to 366".

**Not verified.** The carry layer's fields: the capture holds that the
slot is filled, but not the page its layer is on.

**Then.** Tick 418 stopped where person 21's thread (tree 4113 of
object 52) routes with two people in the way; see "Ticks 418 to 436".

### Ticks 418 to 436

**Who is in the way, in the list's order** (`0x45ba28`, `in_way.rs`).
A trace of tick 418 shows door 52's tree 4105 going to Go To Routing
Slot (tree 4113) in our run. The capture's door 52 holds the same
graphic as ours, so the original takes that branch too. The route then
asks who is in the way of person 21, and two people qualify. The
candidates come from the lot's person vector (`[person +0xdc] +0x84`),
the one Set To Next walks, which the world already keeps as
`World::people`. So they are now taken in that order, and more than one
no longer fails. In tick 418 person 18 has a way but is not asked,
because 18's own route waits on person 21, and person 16 has no way.
Nobody is in the way, the route goes through, and the walk goes on to
door 92, as the capture shows.

**K = 436.** Our state after every tick from 418 to 436 equals the
fourth capture in everything compared, but for the head fields of
"Ticks 334 to 366".

**Then.** Tick 437 stopped where person 16's animator found its next
step blocked; see "Ticks 437 to 466".

### Ticks 437 to 466

**A blocked step** (`0x58eecb` to `0x58f682`, `animator.rs`). When the
person does not fit where the walk's step ends, the route's end
becomes 3 and the update goes to its tail; the person stays put. The
capture's animator holds 3 after tick 437.

**Route step 10, and step 16 on a later run** (`go_to_slot.rs`). In
tick 438 the route sees its animator stopped short and tries again
(step 10 to 6). It finds no way and nobody in the way, and gives up
(step 16). Giving up reads the context's copy of the slot record,
which every run sets (`0x45aec6`), not a null pointer as noted before.
Word 62 takes the planning's result, 13, and the walk goes on to door
87, as the capture shows.

**K = 466.** Our state after every tick from 437 to 466 equals the
fourth capture in everything compared, but for the head fields of
"Ticks 334 to 366".

**Then.** Tick 467 stopped at Get Direction To; see "Ticks 467 to
583".

### Ticks 467 to 583

**Get Direction To** (opcode 12, `0x452003`, `direction.rs`). The
direction from an object to the stack object, in eighths, from the
same function route slots use (`0x4520e9`). It takes the positions in
the native order, +0xf4 first, which is the second coordinate of ours.
In tick 467 person 21's tree 0x1017 looks for a door beside door 93
in a given direction. The direction to object 126 is 0, not the 6 it
wants, so the scan runs out and tree 0x100a draws a Random Number of
2. A capture with a write watch on the random state showed that draw
(return address `0x451edd`, person 21's thread). A first port with
the coordinates swapped found 126, skipped the draw, and left the lot
stream one draw behind with every object still equal.

**K = 583.** Our state after every tick from 467 to 583 equals the
fourth capture in everything compared, the three random streams
included, but for the head fields of "Ticks 334 to 366".

**Then.** Tick 584 stopped at Snap; see "Ticks 584 to 608".

### Ticks 584 to 608

**Snap** (opcode 46, `0x454976`, `snap.rs`). It puts a person straight
onto a place an object names, without walking. Only mode 3 is ported:
the stack object's routing slot named by word 0. In tick 584 person 18's
tree 0x1011 snaps to slot 0 of object 207, one level up. The slot has no
offset and flag 0x800, which skips the placement's own checks of the
border, the room and the walls. The person runs the place test, is put
where 207 stands (level 2, room 1025) and keeps its facing. Its animator
is unset and places itself again. The original goes through floats
(times 3/16 and back times 16/3); for a zero offset and coordinates up
to 0x8000 that round trip keeps the position exactly. A slot with an offset, a snap target, no
flag 0x800, operand flag 0, the other modes, and a person who does not
fit, all fail loudly.

**The upper level's floor test** (`0x4330b7`). Above the ground, an
object without placement flag 0x200 needs a floor on its tile. It
was the step of the place test that kept a person off an upper level;
a person whose person word 84 has bit 0 still fails loudly there.

**K = 608.** Our state after every tick from 584 to 608 equals the
fourth capture in everything compared, the three random streams
included, but for the head fields of "Ticks 334 to 366".

**Stops.** Tick 609: object 260 is removed, and removing it needs the
lot's placement and callback services (the removal of an object that
stands on the lot). See "Ticks 609 to 690".

### Ticks 609 to 690

**The removal queue** (`0x442d70`, `remove.rs`). Remove Object Instance
queues an object as the original does: it skips one that is queued, or
whose holder (each container `capture` gives, up the chain) is queued,
then takes it off the lot through the object's own remove-from-lot
(virtual function 0x50, `Interpreter::take_off`, telling the registry),
and queues it. In tick 609 object 260 removes itself from its tile.
Only while the load has not worked the rooms out does an object leave
the narrower way of `Machine::leave_lot`, which the load was compared
through; it takes an object already off the lot with no callback for
leaving, works its footprint out again as the general way does, and
fails loudly for any other. A tile of a group may stand for its lead tile by
a test (0x42344e, its function table's entry 8) not decoded yet, and
fails loudly.

**No way in the planner's first search** (`0x482abc`). In a room of more
than 256 tiles the planner first searches without the width limit. When
that search finds no path, the context's result becomes 3 and the
planner answers false: the route to the slot drops its candidates and
answers 0. In tick 672 this happens to person 21, whose route then
finds person 16 in the way and asks it to move.

**A random choice of slot places** (`0x483c57`). When the search by
distance finds more places than the context's limit, it keeps a random
choice: the places with word +0xc first, then up to as many attempts as
the limit less those (a signed count; each attempt counts whether or not
it keeps a place, so fewer than the limit can be kept): a draw from the
active stream modulo the number found names a place, and the first from
there on not yet taken is kept, with no wrap past the last. A kept place's
score in the copy becomes 0, which is the mark of a taken place (so a
place that scored 0 is never chosen). Person 16, asked to move in tick
672, chooses this way.

**K = 690.** Our state after every tick from 600 to 690 equals the fifth
capture in everything compared, the three random streams included, but
for the head fields of "Ticks 334 to 366".

**Stops.** Tick 691: person 18 routes in a room upstairs (room 1025),
and the room's rectangles above the ground (`0x4818af`) are not ported.
See "Ticks 691 to 752".

### Ticks 691 to 752

**A room upstairs** (`0x4818af`, `routing/rooms.rs`). The room's
rectangles are worked out the same way on both levels: the room's ID
gives the level (`0x468756`), which picks the room map (`0x481926`) and
the walls (`0x4795c5`, array 102 upstairs), and the tile test
(`0x4821f3`) looks at the objects on that level. The ground's covering
counts only on level 1 (`0x4818e0`), as water does (`0x478f4a`). In tick
691 person 18 routes in room 1025, upstairs.

**K = 752.** Our state after every tick from 691 to 752 equals the fifth
capture in everything compared, the three random streams included, but
for the head fields of "Ticks 334 to 366".

**Stops.** Tick 753: person 21's pose has a carry layer (`0x5992b3`),
which the animator does not port. See "Ticks 753 to 871".

### Ticks 753 to 871

**A carry layer the head does not read** (`0x5992b3`, `head.rs`). A
person carrying something plays the script's carry clip as a layer of
priority 10. The pose is read only for the head: the bones from the
root to the head's anchors and the head itself. A carry layer whose clip
moves none of them leaves them as the foreground put them, so it is
passed over. One that moves such a bone, and a carry clip chosen with no
carry layer, still fail loudly. In tick 753, person 21 carries.

**Set Motive Change** (opcode 29, `0x45a16f`, `motive_change.rs`). It is
the person's own primitive. Flag bit 0 drops all of the person's motive
changes. Otherwise the motive must be 5 to 9, 14 or 15 (native error
0x1a), checked before the amount and the stop value are read. The
change goes on the end of the list: the motive, the amount per hour
times the float at `0x667408` (1/1800) for a change per tick, and the
stop value. The person's tick applies the list in order. Person 18 sets
one in tick 798.

**The head relaxes by a float 0.8** (`0x590886`). With the follow off,
the head's turn shrinks to 0.8 of itself each tick. The code multiplies
by a double (`0x672030`). The original's runs follow the product with
0.8 as a float in each of the five steps of the fifth capture where the
two differ (person 21, ticks 814 to 833), and no step anywhere follows
the double; see the decision.

**K = 871.** Our state after every tick from 753 to 871 equals the fifth
capture in everything compared, the three random streams included, but
for the head fields of "Ticks 334 to 366". Ticks 1 to 608 compare with
the fourth capture as before.

**Stops.** Tick 872: object 121 sets a balloon (Set Balloon/Headline,
opcode 41), which is not ported. See "Ticks 872 to 877".

### Ticks 872 to 877

**Set Balloon/Headline** (opcode 41, `0x44b724`, `headline.rs`). An
object's headlines are a list its construction fills: one for each SLOT
entry of kind 1 (`0x46e2d1`, from the base constructor), and one more
for a person (`0x459630`). Nothing in a house file restores one. The
primitive acts on the first headline of me or, with byte 0 bit 0, of the
stack object (native error 0x15 if there is none); an object with no
headline is left alone. A duration of 0 clears the headline. Showing an
icon (the sprite or picture, its frames, the options, the redraw and the
note to the lot, `0x44574b`, which looks at the scheduler when the
object sleeps) fails loudly. Since nothing here shows one, no headline
ever has a duration, so clearing changes nothing that is kept, and no
headline state is kept yet: the priority is read only while a headline
shows. Sleep's test of the first headline (`0x44aa57`), the countdown
after each thread run (`0x4521f7`) and the reset (`0x45e8af`) are
unaffected for the same reason. In tick 872 object 121, a trash can,
clears its headline.

**K = 877.** Our state after every tick from 872 to 877 equals the fifth
capture in everything compared, the three random streams included, but
for the head fields of "Ticks 334 to 366".

**Stops.** Tick 878: the hour turns (globals 0 and 5, the time of day,
the lighting, signals 0xf8 and 0xdd), which is not ported. See "Ticks
878 to 900".

### Ticks 878 to 900

**The hour turns** (`0x46d3cc` to `0x46d51d`, `tick.rs`). At 60 minutes
the minutes go back by 60 and global 0 (hours) goes up; at 24 the day
turns, which still fails loudly. After the registry's pass and signal
0xdb, the time of day for the new hour (`0x46d525`: night before 6, dawn
at 6, day from 7 to 17, dusk at 18, night from 19) is compared with
global 4. If it differs, global 4 takes it, the interface is told
(`0x4c4648`, the day or night look of the current mode's panel), and
signals 0xf8 (with 5) and 0xdd go out. The lot's subscriber has no case
for 0xf8; on 0xdd (`0x416446`, `0x466ba3`) it sets the brightest a lit
tile may be, which only the drawing reads (`0x6c741c`, `0x4b4525`), and
marks every room for counting and both levels as waiting. The next frame
draw counts them with the new daylight. What the other subscribers do
with 0xf8 and 0xdd is **unverified**. In tick 878 the clock goes from
6:59 to 7:00, but the save already holds day as the time of day (global
4 is 0 at 6:30, in the capture too), so no signal goes out: no room is
marked and the lawn does not grow. The first change of the time of day
is at 18:00.

**K = 900.** Our state after every tick from 878 to 900 equals the fifth
capture in everything compared, the three random streams included, but
for the head fields of "Ticks 334 to 366". Rooms marked by earlier
changes wait for the next frame draw, which in the fifth capture came
after tick 901 (see "Ticks 901 to 991"); the capture holds no rooms, and
the recount shows in what reads it, each person's room motive (taken
every tick) and global 27.

**Stops.** Tick 901: object 8 reads job data indexed by temps 0 and 1
(scope 33), which is not ported.

### Ticks 901 to 991

**Job data** (scope 33, `0x4531fb`, `memory.rs`). My temp 0 names a
career and temp 1 its level; for career -1 the level is 0. The career is
looked up by ID in the list at `0x6de848` (`0x404cb3`; none is native
error 0x39), and a field of the level read (`0x40362f`; no such level or
field is 0x3a), as a value with no word pointer, cut to a word. Field 0
is the number of levels, whatever the level; fields 1 to 21 are the
level's 21 numbers in another order, and field 22 whether its second
string is not empty (`job_field`). The list holds one career for each CARR
chunk of `work.iff`, in resource order, its ID the chunk's ID
sign-extended (`0x403e3f` to `0x403f1f`), with the levels the chunk holds
(`0x4035e7`); `Program::careers` now keeps the levels too. A
`Careers.txt` replaces the list through a loader that is not ported
(`0x4036e8`), and then the scope fails loudly.

**The frame after tick 901.** In the fifth capture person 16's room
motive goes from 19.29 to 36.94 in tick 902, the rooms waiting since
earlier changes (0, 1, 3 and 4 in ours) having been counted before it.
A frame drawn after tick 901 gives that; the default schedule's next
frame, after tick 903, gives it two ticks late. A frame after tick 601 instead gives a room motive in ticks 602 and
603 that the capture does not have, so the fifth capture still drew after
tick 603, and its schedule moved by two ticks somewhere between 603 and
901 (whether it also drew after tick 873 cannot be told: nothing waited).
When the game draws is its main loop's business, so this is the input,
not the simulation: comparisons with the fifth capture past tick 901 draw
`0,1,2,3,33+30,901`.

**K = 991.** With that frame, our state after every tick from 901 to 991
equals the fifth capture in everything compared, but for the head fields
of "Ticks 334 to 366".

**Routing slots are the SLOT entries of kind 3.** An object's routing
slot table (`+0x13c`) holds only its SLOT chunk's entries of kind 3, in
order, each numbered by its index in the chunk plus one (the parse at
construction, `0x46e2d1`, which sends kind 0 to the containers and kind
1 to the headlines). Go To Routing Slot and Snap named a slot by its
index in the whole chunk, which is the same for every object the Goth
house's routes reached before; they now count among the kind 3 entries
(`Program::routing_slot`). Object 174's routing slot 0 is its SLOT
entry 1, which snaps into the object.

**Snap's checks of the place** (`0x4550d8` to `0x4551e9`, `snap.rs`).
Without slot flag 0x800 the placement first checks the place: its tile
inside the lot's border (else placement error 1), the room its tile's
cell names the same as mine (object data word 29), and, when I stand
inside the border and elsewhere, the direction to it open from my tile
and the way back open from its tile (`0x4359ea`). A refusal fails loudly
with the rest of not fitting.

### Ticks 992 to 1000

**Snap into a container slot** (`0x454b19`, `snap.rs`). A routing slot
whose snap target is 0 or more puts me into that container slot of the
object. Scripts count an object's container slots from its saved slot 1
(slot 0 is its tile's chain), so one past the last is native error 0x20.
The place is the object's position and level, and the place test is the
one for a slot (`0x432d62` to `0x432fb8`, `fits_container` in
`placement/find.rs`), refusing at the first of these that fails:
neither object has data word 8 flag 4; the slot exists; it holds nothing
or me; in mode 2 (global 19) an object without flag 0x2000 needs a slot
at the centre, or a container with no other slot; the other slots at the
same height that hold something leave room (a centre slot's object needs
flag 0x2000, and so does mine for a centre slot); the container's data
word 28 is at least my data word 27; I am neither the container nor
anything around it; and my data word 4 has the bit for the slot's
height. Before these, the test's first steps on the ground run, without
the wall rules. A container slot off the object's centre (an offset
across that is not zero) would also move my animator offset
(`0x454ceb`), which is not kept, so it fails loudly. Snap's flag bit 2 (test the walls around the object
instead, `0x45522c`) and a person as the container fail loudly.

**The base clips of postures 1 and 2** (`0x590497`). When a person's
posture becomes 1 or 2, the animator starts the posture's base clip:
clip 1 or 2 of the posture's clip names (posture 0's is clip 3). The
clip is looked up (`0x5904af`, `0x59dc32`); one that is not found, or
that has timed events (which the skin's update would play, `0x5992b3`),
fails loudly. Posture 0's clip is still not looked up, as before: the
captures always show its layer after a start.

**K = 1000.** In tick 992 person 21 sits: it snaps (tree 0x2017 node 5)
into container slot 0 of object 174 (GUID 0x24E25E86), its posture
becomes 1, and its base clip starts; it sits there through tick 1000.
Our state after every tick from 992 to 1000 equals the fifth capture in
everything compared, but for the head fields of "Ticks 334 to 366"
(the target height of persons 16, 18 and 21, one unit in the last place
lower). The fifth capture ends after tick 1000.

## Track: drawing people (`crates/sims-people`)

Draw the Sims from the original's files, posed by the simulation's
animator state, which the crate reads and never changes. Milestone: the
Goth family's three Sims drawn with correct meshes and skins, posed from
a skeleton, playing an idle and a walk clip in an offscreen render and a
small viewer, with one pose compared with a screenshot of the original.

Progress:

- Done: the person files are found by name across the base game and
  expansions; a saved person's appearance strings choose the meshes and
  skin textures; the skeleton is posed and the meshes skinned on the CPU;
  a software rasterizer draws them offscreen. The three Goths draw in
  their rest pose with the right meshes and textures (pinned by digest,
  gated on `SIMS_INSTALL`).
- Done: animation clips and their samples load by name; a clip is posed
  at the phase the simulation's foreground layer gives, over the
  posture's standing loop; a person is placed and turned from the
  animator's position and direction. The Goths stand and walk offscreen
  (pinned by digest).
- Done: a small viewer (`people-viewer`, behind the `viewer` feature)
  plays the house's people standing or walking, or draws them where a
  simulation snapshot has them; it can also write one frame to a PNG.
- Done: one pose compared with the original. Bella walking, seen from
  behind in the emulator, matches in meshes, textures, proportions and
  pose (see the decisions entry). Appearances are also found by their
  own names across every catalog.
- Done: the drawing is the original's image, not its mirror (Cassandra's
  pocket protector, seen from the front); bones compose exactly as the
  original's code does; the lot lies on the screen as the lot drawing has
  it. Catalogs load in the original's order, and the head turns as the
  simulation says.
- Done: the viewer runs a booted lot (`people-viewer --boot`, through
  `sims_vm::boot::Lot`) and draws its people after each tick, including
  people a script makes; it only reads the lot. A gated test runs the
  Goths' lot until a step stops it (or 3000 ticks) and draws everyone
  placed every 30 ticks, centred on the root bone, which a clip such as
  the stair climb carries away from the animator's position.
- Done: people in the lot renderer's view (`sims_people::People`,
  `View`). An app keeps one `People`, calls `update` with the live world
  each frame (new people and changed clothes are loaded, people who left
  are forgotten), then `draw` with the lot renderer's camera, any zoom
  and rotation, into any `raster::Target`, with a depth function shaped
  like the lot renderer's `Depth::point`. A person stands on the terrain
  under them. A gated test draws the Goths' lot's people at all four
  rotations every 30 ticks and checks each is drawn where the lot
  renderer projects them.
- Done: people drawn into the lot renderer's canvas, tested against the
  lot's depth (`People::draw_on_lot`, after `sims_render::draw`): walls,
  furniture and the floor above hide them. A gated test draws each of
  the Goths' people alone over their lot every 120 ticks at four
  rotations and checks that something hides them at times, that their
  feet show in most views, and that nobody is hidden throughout.
- Next: compare a person behind furniture with the original; a person on
  an upper level stands at the floor, without the original's offset.
  Match the original's lighting (see its decisions entry); compare
  Mortimer standing without his book. The base layer's clip and phase
  are not simulation state yet, so the saved record and the animator's
  clock stand in.

## Track: lot rendering (`crates/sims-render`)

Draw a lot from the house file and the game's art: terrain, floors, walls
and object sprites, at the three zooms and four rotations, in a window.
The crate reads the simulation's world and never changes it. Milestone:
the Goth house drawn in a window at every zoom and rotation, with floors,
walls (with cutaway) and objects, and one camera compared with a
screenshot of the original.

Progress:

- Done: the crate draws on the CPU into an image with a depth buffer,
  using the original's projection. Floors, grass, walls (with the shapes
  doors and windows cut into them) and object sprites draw at every zoom
  and rotation, with walls up, cut away or down. The Goth house renders
  are pinned by digest, gated on `SIMS_INSTALL`.
- Done: `sims-viewer` shows a house (or a world saved by `sims boot
  --snapshot`) in a window, with keys and the mouse for zoom, rotation,
  floor, the three wall modes and scrolling, and saves frames as PNGs.
- Done: one camera (far zoom, rotation 0, floor 1, cutaway) compared
  with a screenshot of the original. The projection and outdoor sprites
  match pixel for pixel, and two wall bugs it showed are fixed. See the
  decisions log for what still differs.
- Done: rooms darken by their light, as the original draws them; at the
  compared camera, exact non-grass matches rose from 34% to 42% (cutaway)
  and from 40% to 47% (walls up). Lamp pools and light levels other than
  the half-light floor are unverified.
- Done: the original's grass, shading and blades read from its code;
  open grass matches the original pixel for pixel at the compared
  camera, and exact matches over the whole screen rose from 18% to 84%
  (walls up). Other zooms and rotations are unverified.
- Done: the cutaway cuts the rooms the original looks at: the last
  three rooms the pointer passed over and the selected Sim's. Doors and
  windows hide with their walls by the same mask. At the compared
  camera, the house box rose from 61% to 66% exact (67% on a booted
  world).
- Done: the wood trim along the tops of standing and cut walls; at the
  compared camera the house box rose from 72% to 76% exact (walls up)
  and from 67% to 69% (cutaway).
- Done: two more cameras compared with the original. At medium zoom
  (rotation 0) open grass matches 96% and the house 74%; objects set
  into walls are a pixel or two off. At far zoom, rotation 3, the house
  lines up but the grass matches only 13%; why is not known yet.
- Done: the grass at other rotations. The sun no longer turns with the
  camera, as in the original. At far zoom, rotation 3, open grass rose
  from 13% to 99% exact, and the whole screen from 23% to 81%.
- Done: objects sit one pixel right of the point the camera projects
  them to, as in the original. At the three compared cameras the house
  box rose from 74% to 89% (medium zoom), 76% to 89% (far zoom,
  rotation 0) and 74% to 89% (far zoom, rotation 3).
- Done: objects held by others (lamps on end tables and counters, the
  sink in a counter) stand on their holder's slot. The lamps were drawn
  on the floor, inside their tables.
- Done: wall caps, as the original's corner records lay them: a narrow
  slice of wall closes each exposed wall end, and a cut wall meeting a
  standing one keeps a full-height stub with a trim edge. At the
  cutaway camera the house box rose from 84% to 86% exact; the walls-up
  cameras rose by up to half a point.
- Done: light levels use the original's colour tables, which shift blue
  by a step; at the compared cameras the house box rose by 0.7 to 1.3
  points (medium zoom 90.4%, cutaway 87.1%).
- Done: far zoom at rotations 1 and 2, and near zoom, compared with the
  original. The geometry lines up at all of them; the house box is
  56.2% to 90.4% exact. Most misses are one room the original lit
  brighter (the shots are from later in the morning) and the Sims; an
  edge-on diagonal wall's texels also differ.
- Read, not ported: the original's strip on a marked edge-on diagonal;
  no screenshot shows one.
- Done: depth in the original's own values for grass, floors, walls,
  trims and objects, checked against the game's depth buffer (99.1%
  medium, 99.7% far, 97.2% at rotation 3). Faint sprite edge pixels
  are left out as the original does, which fixes the door jamb. Every
  compared camera rose, by 0.7 to 3.4 points.
- Done: the API for drawing people over the lot. `Depth::point` gives
  a point between pixels its depth in the same space, `Canvas::put`
  draws one pixel by the sprite rule and `Camera::project_fine_f32`
  projects without rounding. Drawn this way after the lot, people are
  hidden by what is in front of them, as in the original, whose depth
  buffer holds the lot without its people. The game does not use it
  yet, and how the original gives a person's mesh its depth is
  unverified.
- Done: the bay's edge-on ends. The original draws an edge-on diagonal
  wall 300 nearer, anchored halfway along it (0x4c7be2, 0x4c4907), so
  the straight bay walls' side faces show in front of it. At near zoom
  both bay ends now show the shingles (61.2% to 61.7% exact); at
  medium zoom the depth at both ends is the original's.
- Read, not ported: the side an edge-on diagonal shows. The original
  shows the side facing view (−1, 1) (0x53f6c1, 0x53d93a); ours shows
  the same lot side at every rotation. The one column where the wall
  shows matches neither side until its texel offset is read.
- Done: which object or person drew each pixel, for clicks:
  `Canvas::id_at` after `sims_render::draw` and `People::draw_on_lot`.
  Objects name themselves, people their object ID, and terrain, floors
  and walls nobody. The game window picks by it.
  How the original finds what a click hit is not read.
- Done: the pools of light lit lamps throw on the floor, from the
  game's `lightMap.dat`, and lit lamps drawn in full light, as the
  original does. With the world at the screenshots' time, near zoom
  rose from 85.2% to 86.1% exact and far zoom at rotation 2 from 89.9%
  to 90.1%. Pools on walls (the file's wall tables) are read in part,
  not ported.
- Done: lamp pools on straight walls, from `lightMap.dat`'s wall
  tables (grass takes none in the original). Far zoom at rotation 2
  rose from 90.1% to 90.3% exact; of the pixels they change, 70% now
  match exactly and the rest are mostly a step off.
- Next: near-zoom fidelity below the bay's left end (rows 436 to 520,
  grass and a light sprite in the original); the
  edge-on texel offset and side; a screenshot with a marked edge-on
  diagonal; upper-level depth.

## Track: sound (`crates/sims-audio`)

Goal: the game's sound, from files to PCM, in `crates/sims-audio`. The
simulation never depends on it; its only link is the play-sound and
stop-sounds events.

- **Done: every sound file decodes.** XA (18,155 files), WAV (285) and
  MP3 (141) from loose files and FAR archives, by the paths the sound
  tables use. XA and WAV match FFmpeg bit for bit on every file. No UTK
  file exists in the install.
- **Done: the sound tables.** All 27 modules (HIT programs, HSM
  symbols, HOT tables) and the 4 FSC ambient scores parse. Every HIT
  program disassembles, and every entry point and branch resolves. The
  files' own loose ends (155 missing samples, among others) are counted
  in a gated test. The phone ring and a footstep trace to their samples.
- **Done: the HIT VM.** `Player` takes `SoundEvent`s and renders PCM.
  The phone ring and a soft footstep render their samples bit for bit,
  and every event definition in the install (4,494, module by module)
  runs without a bad or unported instruction. Sequence groups and fades
  follow the original's code. Scripts cannot hang or exhaust the player.
  Volume, pan, pitch and ducking follow the original's formulas, and
  the output is stereo. The `render_event` example writes events to a
  WAV file.
- **Done: event kinds.** Fades, television and radio stations (built
  from their directories, as the original does), sequenced hit lists,
  `radio_next_song` and the kill events follow the original's
  dispatcher. Of the 190 events that were not plays, 29 remain, counted
  in a gated test: music modes (21), ambience modes (5), the piano,
  pausing and reloading. 12 more name no track and do nothing.
- **Done: ambient scores.** The FSC scores play as the original's score
  player does: cells picked by weight in a window around a cursor that
  the camera and the hour move, on the score's beat. The ambience modes
  (52) and the ambience half of the music modes (36) are ported, so 24
  events remain.
- **Done: live playback.** The `play` example plays events (and the
  ambience) on the default output device, behind the `playback`
  feature (cpal), off by default; tests and CI build none of it.
- **Done: the ambience's inputs.** The simulation emits its sound
  requests (`Update::sounds`), and the game plays them. The camera
  value the ambience's row follows is ported (`ambience::view`, from
  0x570c3e: the share of the screen's tiles that are outdoors, read
  through `World::outside`), and the hour is the clock's.
- **Done: pause and unpause** (21 and 22): the sounds playing are held
  where they are, but for the interface's and the main music's, and the
  live ambience stops picking cells.
- **Done: what scripts ask about an object.** `getsrcdatafield` gets the
  original's answers (0x570e61: a person's voice, age, skills,
  personality and motive 3, the hour, the size of the object's room),
  read from the lot between ticks (`sims_game::sound::sources`).
- **Done: where a sound's objects are.** Kinds 1 and 30 set a track's
  volume and pan from where its objects are on the screen (0x5730b6,
  0x57473b, 0x570e09), which the game sends with the answers, from its
  camera.
- **Done: the music modes** (36). A change of mode starts and stops the
  music tracks as the original's sound manager does (0x575170): the
  buy, build and neighbourhood lists, the expansions', the credits', the
  river loops and the slow fade into live mode. `play_trk` hands on the
  caller's arguments and `source`, and `kill_trk` ends the whole track.
  3 events of kinds not ported remain: the piano (43), and 103 and 104,
  which are debug commands.
- **Next:**
  - The piano (43).
  - Placing the tracks again when the camera moves (kind 33), once the
    game sends it only then.
  - Tracks and fades step once a block (64 frames, about 2.9 ms); the
    original's sound manager steps them once a millisecond (0x57cf37).
  - Checking any of it against the original.

## Track: recording tool input (`tools/harness`)

Drive the original from a script while it is captured: click objects,
choose pie-menu entries, enter and use build and buy modes, confirm each
input from guest memory, and record it with the lot update it falls in,
so our simulation can replay it. Milestone: a capture where a scripted
input makes a Goth Sim do a chosen interaction, confirmed from memory and
recorded with its update; then buy mode and placing one object.

Progress:

- Done: `capture.py --scenario` runs a scenario file (`tools/harness/input.py`,
  documented in the harness README). Input is xdotool in the guest; the
  game's state is only read. From a fresh boot, a scenario answered the six
  welcome dialogs, clicked the porch bench, chose Sit, and the pie menu's
  push of Sit on bench 173 for person 21 was recorded before lot update
  178. The full state after update 200 holds it as person 21's active
  action.
- Done: buy mode. From a fresh boot, a scenario entered buy mode (mode 2,
  read at update 174), took the $80 chair from Seating, put it on the lawn,
  and went back to live mode (mode 0 at update 1078). The new object (ID
  262) and the budget's fall from 2542 to 2462 were read at update 878.
- Next: build mode (its mode number), and a replay of the recorded push and
  the purchase in our simulation. The replay needs a way to queue an
  action on a person, and to place a bought object, before a tick.

## Track: the interface (`crates/sims-ui`)

Goal: the game's interface drawn from the original art: the live-mode
control panel, dialogs, pie menus, and later the other panels and the
catalogs. The art goes in the original's layout, adapted to any window
size. The crate only reads the simulation. Every click becomes a
`sims_ui::command::Command`, and the host decides what to do with it.
Drawing goes into a CPU frame, so `cargo test` needs no window.

- **Done: the interface's art.** `sims-formats` reads its BMP and TGA
  files and `sims-content` reads its resource tables (#62).
- **Done: the live control panel.** The cluster (modes, camera, speed,
  clock and funds) and the strip (portraits, mood bars, the gadget and
  the Needs panel) are drawn at any window size, from a loaded world.
  Measured against a screenshot of the original at 800×600; the
  numbers are in the decisions log. The Goths' panel is pinned by
  digest at 800×600 and 1280×720, gated on `SIMS_INSTALL`. `sims-viewer
  --hud` shows it over the lot. Camera buttons move the camera; the
  other commands are printed, since the simulation cannot take them yet.
- **Done: message dialogs.** The dialog a script's Dialog instruction
  shows (one, two or three buttons) is read from the record and the
  caller's string table and drawn with the original's fonts, icon and
  button art. Measured against two screenshots of the original
  ("Magic Town", "Studio Town"). Inside the frame they match except for
  5 pixels, and the button except for its white label under the pointer.
  The frame's corners are close but not exact. "Magic Town" is pinned by
  layout and digest, gated on `SIMS_INSTALL`. `sims-viewer --hud` shows a
  waiting dialog, and its answer is printed.
- **Done: the pie menu.** `sims_ui::pie` lays out, hit-tests and draws
  the menu and groups an object's actions into submenus, ported from the
  original's menu window and object picker. The window and label boxes
  agree to the pixel with two menus read from the running original
  (gated on `SIMS_INSTALL`); the drawn look is not yet compared with a
  screenshot. The host keeps a `pie::Pie` and renders the head.
- **Next:** open houses through `sims_vm::boot::Lot` (which now runs the
  household load) in the tests and the viewer, in place of their
  stand-in for steps 17 and 18; a screenshot of an open pie menu; a
  Yes/No dialog checked
  against the original (the "Help System" one at tick 2) for the layout
  of several buttons; text entry dialogs; the other live panels.
- **Needs from `sims-sim` and `sims-vm`** (this track does not add them):
  - A command intake, for example `World::apply(&mut self, command:
    sims_sim::Command) -> Result<(), CommandError>`, run between ticks,
    taking: select a Sim (object ID), set the speed, pause or run, go
    to a mode, answer a dialog (caller, button), and choose a pie menu
    entry (Sim, object, interaction).
  - The household's member order kept in the world, for example
    `World::household(&self) -> &[u32]` (GUIDs), so the portraits do
    not need the neighborhood file.
  - The game speed, for example `World::speed(&self) -> u8`.
  - A way to leave a dialog waiting: `sims boot` answers each dialog as
    it is made, so no snapshot holds one for the viewer to show. For
    example, an answer policy in the boot runner that stops at the first
    dialog without an answer and leaves it at status 0. The intake then
    answers it.
  - The frame that ran the Dialog instruction, for example
    `Dialog::owner(&self) -> u16` (the code owner's type). Today the
    interface takes the caller's top stack frame, which is right only
    while the caller waits on the dialog.
  - Each Sim's current action and its icon, for the icons over the
    portraits.

## Track: integration (`crates/sims-game`)

Goal: a first human-playable Goth house. One window runs
`sims_vm::boot::Lot` at the original's tick rate and draws the lot
(`sims-render`), its people (`sims-people`) and the live-mode panel
(`sims-ui`); clicks on objects open pie menus that queue interactions;
the panel's speed, pause and camera buttons work; the simulation's sound
events play through `sims-audio`. Done when a human can launch it, click
the fridge or the bookshelf, and watch Mortimer go and do it, with sound,
for several minutes without the lot stopping. The crate joins the others
through their public interfaces only; the simulation never depends on it.

- **Done: the window.** `cargo run --release -p sims-game` boots the
  Goths' house (`SIMS_INSTALL`, `House05.iff`) and runs it in strict
  mode at the original's pace (below). The lot, its people (hidden by
  the walls, furniture and floors
  in front of them) and the panel over both are drawn every tick; the clock, funds and needs on the panel are the live
  world's. The panel's speed, pause and camera buttons work.
  The first error stops the lot, which stays on screen as it was with
  the failure across the top. `--play` runs it in play mode instead: an
  object whose tick fails is reset and the lot goes on, and each new
  error is printed and shown across the top for a while; only a failure
  play mode does not catch stops it. In play mode the Goths' house runs
  12,000 ticks (ten minutes at normal speed) without stopping, through
  about a dozen different caught errors; in strict mode it stops after
  tick 3602 (at tick 3603 Create New Object Instance mode 6 is not
  ported).
  `--png <file>
  --ticks N` draws one frame without a window.
- **Done: sound output.** The window plays `sims-audio`'s player on the
  default device (on macOS; elsewhere with the `sound` feature, so CI
  needs no audio library; `--mute` turns it off). A thread renders
  22,050 Hz stereo ahead of the device and the game never waits on it.
  The ambience follows the clock's hour (global 0), and the sound
  scripts' speed register follows the game's speed.
- **Done: the lot's sounds.** Each step's sound requests
  (`Update::sounds`) go to the player as the window wakes: a door opens,
  a book comes off the shelf, the fountain tinkles. `begin_route` and
  `end_route` have no event in the install's sound files and play
  nothing.
- **Done: the pie menu from the simulation's listing.** When the
  pointer first reaches an object or a person, the game lists the
  selected Sim's menu on it with `Lot::pie_menu`, which changes the lot
  as the original's listing does; a click opens it. The entries are
  those whose check passes, named as the check tree named them or else
  by the TTAs strings of the table they came from, with "*" before such
  an entry with flag 0x80; choosing one queues the offered action with
  `Lot::pick`. Where the simulation has
  not ported something the listing needs, the menu falls back to every
  entry of the table, queued by menu number, and the note across the
  top says why; any other failure shows no menu. Escape closes the menu.
  The menu is drawn as the original's pie, with `sims_ui::pie` (#157):
  the labels around the click, a name with a `/` in a submenu, at most
  12 shown; a test clicks the fridge and chooses Have A Snack from its
  pie. In its middle is the head of the Sim the menu is for, as posed
  now, drawn by `sims-people`'s rasterizer from in front (its camera,
  light and size are ours, not compared with the original's). The simulation lists the fridge itself: in the morning the house
  loads in, Have A Snack, Have Quick Breakfast, Have Breakfast and
  Serve Breakfast.
- **Done: a Sim doing what he is told.** Have A Snack queued on the
  fridge at tick 51 (the snack order, run 2026-11-15 on main d46c93ab,
  in play mode and in strict mode alike): Mortimer takes the order at
  tick 150, the fridge makes the snack (object 263) at tick 615, he
  takes it at 748, pays 5 at 820 and eats it (menu 2 on the snack) while
  his hunger goes from 94.5 to 100.0 (tick 936); at 1088 the snack goes
  off the lot and at 1136 it is deleted, and he is free again. He goes
  on to other things (object 71 at tick 1497, object 132 at 2292).
  Strict mode now runs the order through step 3602 and stops at step
  3603 (object 354 uses Create New Object Instance mode 6 with flag 0x4,
  not ported); without an order it stops there too.

  not ported); without an order it runs to tick 3432.
- **Done: a player's order from the window, in play mode.** With
  `--play`, the pointer on the fridge lists its pie, a click opens it,
  and Have A Snack queues the action the listing offered. A Goth test
  drives this through the game's input layer: the action queued is the
  scripted order's, and from step 150, when Mortimer takes it, the lot
  is the scripted lot hash for hash through the snack's end at step
  1136, with no error caught.
- **Done: the speed and the pause.** The panel's speed buttons set the
  simulation's speed as the original's setter does (`0x46d697`, globals
  16 and 18, `sims_sim::speed`): 760, 924 and 994. The window ticks at
  the setter's period for that speed: 42, 14 and 2 ms. The pause button
  and space set global 17 (`0x46d813`, `0x46d83a`), and the lot goes on
  ticking with its clock standing, as the original's does: every pass is
  a partial one. The Goths' house runs 200 such ticks in strict mode.
  The idle loop's own speed is ported too (`0x46d9d6`,
  `World::idle_speed`). On a residential lot where nobody holds the
  speed (`0x442f9b`, presumably everyone away or asleep), it runs at
  1000 and keeps the speed to go back to in global 18. It goes back once
  someone holds it. The window follows: 1 ms a tick at 1000.
- **Done in play mode: selecting a Sim.** A press on a portrait selects
  that Sim (0x445449, between ticks): the Sim selected before is
  deselected, the indicators move, and the panel and the pie menus
  follow the new Sim. Deselecting refreshes the old Sim's animator
  (0x58da12), which is not ported, so in strict mode (`Lot::select`)
  switching Sims fails and changes nothing; play mode
  (`Lot::select_play`) leaves the refresh out.
- Done: a click picks what the frame shows at the pixel
  (`Canvas::id_at`, `sims_game::pick`): the person shown there or
  nearest within two pixels, else the object shown there. A tile of a
  multi-tile object is picked as itself; the simulation sends its
  interactions on (0x435286). How the original finds what a click hit
  is not read.
- **Done: answering a dialog from the window.** A dialog with a choice
  waits for the player: the lot stops before the tick that shows it,
  the dialog is drawn from the world as that tick left it, and a press
  on a button answers it and the tick runs again with the answer
  (`Lot::wait_for_answers`, `Lot::answer`). The Help System's dialog at
  tick 2, answered No from the window, gives the lot `--answers 3`
  gives, hash for hash, through tick 200. `--answers` still answers
  first, and `--png` answers No to the Help System by default.
- **Done: the panel follows the lot.** Every frame the panel reads the
  world as it is: the clock (globals 0 and 5), the household's funds,
  the portraits with the selected Sim's lit, and the selected Sim's mood
  bars and needs meters. A Goth test queues the snack and runs 850 ticks:
  the funds go from $2,542 to $2,537 when Mortimer pays, the clock moves,
  and the hunger meter is his hunger. The pointer on a portrait shows
  that person's first name (catalog strings, `OBJD` word 41) in a box
  over it; that box is ours (**unverified**).
- **Done: the action queue.** The selected Sim's running and queued
  actions show as icons in the top left, framed as the original frames
  them, and a press cancels one (`Lot::cancel`, 0x45d5f8) with the
  original's sound. Objects' catalog pictures are not made yet (a grey
  square stands in), and the icons do not slide into place.
- **Needs from other crates:**
  - `sims-vm`: listings that fail today on the Goth house: check trees
    on Manage Inventory operation 4 and scope 12. A check tree's fault
    should keep its trap rather than become `Unsupported` text, which
    `sims-game` reads to tell unported from failed.
  - `sims-vm`: a play-mode step that fails should still hand
    back the errors it caught before failing, and say which frame
    (before or after the tick) was skipped.
  - `sims-vm`: the animator's refresh (0x58da12), which deselecting a
    Sim makes, so strict mode can switch Sims.
  - `sims-people`: the selected Sim's head as an image, which the
    original renders live into the pie's face bitmap; the middle of the
    pie is empty until then.

## Working rules

- Work in branches, open PRs, and merge when CI is green on both platforms.
  Squash merges only, no force-pushes, and run
  `scripts/check_repository.py --history HEAD` before every push.
- Nothing from the original game is committed. Fixtures that need game data
  stay private and are skipped when the data is absent.
- Choices about format details, unverified guesses, and the order of work
  are made in the PR and recorded in `docs/decisions.md`. Guesses are
  marked unverified in the code until the harness confirms them.
- Stop and ask the maintainer only when:
  - something only they can provide is needed (a login, or files not on
    this machine);
  - a choice would change `docs/vision.md` or the product scope;
  - something destructive outside the repository is needed;
  - the original cannot be observed in the emulator closely enough for the
    harness, which means the strategy has to change.
