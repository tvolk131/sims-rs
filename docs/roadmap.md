# Roadmap

The current goal and how work toward it runs. `docs/vision.md` says what we
are building. This file says what we are building *next*, and when it counts
as done.

## Current goal: a headless lot boot that matches the original

Point `sims` at an install. It finds the files, loads one base-game
neighborhood house, runs the objects' setup scripts, and then runs N ticks
with no window. Every run gives the same state hash, and the state matches
the original game, run in the emulator, for the first K ticks.

Done when:

- `sims boot <install> <house> --ticks N` prints a state hash that is the same
  on every run and on both CI platforms (x86_64 Linux, aarch64 macOS).
- For that house, a captured run of the original agrees field for field with
  ours at every compared tick up to K. K starts small and grows. The goal is
  met at K = 1000 ticks, with no unexplained differences.

### Steps

Each step is a few PRs, and each PR leaves `main` green.

1. **Install detection and content lookup.** Recognize a Complete
   Collection install (from its ISO) by its files. Bind every game file and
   resource by name, and fail loudly on anything missing. The layout is
   checked against the real installed tree in the emulator image.
2. **Lot loading.** Read a house file into simulation state: objects, their
   attributes, the scheduler, both RNG streams, the budget and the lot
   arrays. Snapshots save to and load from REDB and survive a round trip
   bit for bit.
3. **Script interpreter core.** BHAV execution with stack frames, the
   operand and variable model, and the primitives that this lot's setup and
   first ticks actually reach. Unimplemented primitives fail loudly and
   name themselves. Arithmetic goes through `sims-core`.
4. **Comparison harness.** Run the original under QEMU and Wine, load the
   same house, advance to tick N, and dump the state we compare. Captures
   are private and are not committed.
5. **Match.** Diff our state against the capture, fix the causes one by
   one, and raise K.

### Progress

- Step 1 is done: `sims install` detects a Complete Collection install and
  builds the object catalog.
- Step 2 is done: the Goth house reads into `World`, which has a state
  hash and REDB snapshots. `sims boot <install> <house>` prints the hash of
  the state its load reaches. `--ticks` waits for step 3.
- Step 3 is in progress. The interpreter runs saved frames, creates
  objects through the original's factory, and puts them to sleep. The load
  sequence below tracks the load steps that must run before the first
  tick; `sims boot` runs them through step 9.

### Load sequence

What the original does between opening a house file and running the first
tick, in order, and how much of each step `sims boot` does. The goal
above needs every step: a step skipped or run out of order
changes the state the first tick sees.

The order and the addresses come from the old prototype's research on the
1.91 executable. Each step was checked there on its own, against the
original's code run in isolation. Steps 1 to 6, 8 and 9 are also checked
against captures of the running original (see steps 3, 5, 6, 8 and 9), and
three results of step 7 are (see step 7). The rest of the sequence has not
been compared with a running game, so it is **unverified** until it is
ported and compared.

The original does not build a lot for a house. It keeps one lot for as long
as it runs and loads each house into it. So a load starts from whatever
that lot holds: at least the two indicator objects, which every load
creates at its end (step 14). `sims boot` has no earlier load, so it starts
from an empty lot (`World::empty`) and creates the two indicators on it.
That starting lot is **unverified**; see the decision log.

| # | Step | Original | Status |
|---|------|----------|--------|
| 1 | Read the house's simulator state into the lot the game is holding | `0x46cfb3` | Done (`World::read_simulator`) |
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
| 10 | Give every object its room | `0x442659` | Not started; needs the lot's rooms |
| 11 | Discard pending removals | `0x428679` | State change done (`World::discard_removals`), not called yet |
| 12 | Select the household | `0x428041`, `0x46d8dc` | State change done (`World::select_family`), not called yet |
| 13 | Visit every object in registration order | `0x43fc94`, `0x431e2e` | Not started |
| 14 | Create the indicator objects and finish selection | `0x445e4a` | Creation done (`Interpreter::create_indicators`), used only for the starting lot |

`sims boot` runs the steps in order and stops at the first one that is not
done, so it never prints the hash of a state the original could not be in.
Today it stops after step 9.

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
lot all fail with an error that says so. The rooms themselves are not
modeled.

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
4, 6 and 7, or any cell outside the 52 tiles; those are **unverified**.
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

**10. Rooms.** Two passes over the registration list. The first writes each
object's room into data word 29, turning level 0 into level 1. The second
runs each object's function 7.

Not ported: it needs the lot's rooms, which are worked out from the walls.
What the capture of the Goth load shows of this step, for whoever ports
it: the lot's rooms are built between the end of step 9 and the start of
this step, 11 of them where the lot had 2. During the step no data word of
any object changes, so every room the first pass writes is the one the
house saved. Function 7 is asked of all 382 objects in registration
order. The last result of 62 of them changes, and none of the other
fields step 8 compares. After the step and before step 11, global 27 (the
lot's indoor area) goes from 0 to 204.

**11. Removals.** The queue of objects waiting to be removed is emptied.
The objects stay. In the capture the queue holds four IDs before and none
after, and no object changes.

**12. Household.** Global 9 takes the family number, global 10 the house
number, and the family's money becomes the lot's funds, split across
globals 13 and 14.

**13. Object visits.** For each object, in registration order:

1. A person takes its persistent fields from the neighborhood record
   (`0x4258a3`).
2. Its position is restored, which also places the objects it contains
   (`0x433e50`).
3. A non-person runs its Load function (2). A function whose tree is
   missing is skipped.
4. If its type changed: every person drops references to it (`0x444306`),
   then it resets (`0x431fbe`). An object with a Reset function (3) runs
   it and restarts Main. One without is taken off the lot, has its storage
   reset (`0x431be1`), runs Init, and is put back. A person has its own
   reset (`0x45e791`).
5. If its type changed, it runs Init.
6. Its scheduler flags are refreshed from data word 60 and its
   interaction table.
7. A person updates its cached room and reloads its motive curves.

Lot time does not advance, and no object's saved Main runs, during any of
this.

**14. Indicators.** Two indicator objects are created through the factory
and hidden, and the selected person is found.

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
