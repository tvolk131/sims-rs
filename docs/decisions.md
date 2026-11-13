# Decisions

Choices made while working toward `docs/roadmap.md`, newest first. Each entry
says what was decided, why, and what would reopen it. Entries marked
**unverified** wait on the comparison harness.

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
