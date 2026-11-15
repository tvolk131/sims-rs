# Saved objects are constructed and then read, and the load runs through the object read

*Decided 2026-11-13.*

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
