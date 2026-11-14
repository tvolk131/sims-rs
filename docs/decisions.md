# Decisions

Choices made while working toward `docs/roadmap.md`, newest first. Each entry
says what was decided, why, and what would reopen it. Entries marked
**unverified** wait on the comparison harness.

## 2026-11-14: The comparison harness runs again, and a capture holds 1000 ticks

The original runs again in the emulator and can be stopped and read at
every lot update. `tools/harness/` holds the scripts. A new private
capture of the Goth house holds the state at the entry of tick 1, after
each of the first 100 lot updates, and after every tenth up to 1000.

What was decided:

- **The harness is the old prototype's, cut down.** Debian bookworm
  with `qemu-system-i386` in Docker. An i386 guest with full emulation
  (TCG) runs `Sims.exe -w -nosound` under Wine. Hardware breakpoints
  stop the game at the entry of the lot update (0x46d345) and at the
  end of its body (0x46d51d). State is read through QEMU's GDB stub.
  Nothing writes guest memory or registers. The only inputs are the
  button presses the README lists.
- **The guest reads the install's own UserData.** The guest's UserData
  folders are replaced with copies of the install's, and their hashes
  are checked before the game starts. With that, the C runtime stream
  and all 463 neighborhood records match ours at the end of the load.
  The first capture's 52-draw offset and its five differing records
  came from that guest's own files, not from the code.
- **Dialogs are answered as in the first capture.** OK to the five
  notices in tick 1 (read with status 2), and No to the Help System in
  tick 2 (status 3). The game closed two later dialogs itself, in
  updates 901 and 902.
- **Captures stay private.** They live in the old checkout's ignored
  `cache/`. The guest disk, kernel, initramfs and ISO are inputs that
  the repository does not hold.

How it was checked: our state at the entry of tick 1 and after each of
ticks 1 to 5 was compared with the new capture field by field. 248
lines are equal and 3 differ:

- Registry byte 0x12 is already 1 when the original's load ends. Ours
  sets it in tick 2. This is two of the lines (load and tick 1).
- After tick 1, global 12 is 17 in the original and 0 in ours. Global
  12 is the interface's wait check, which we do not run. The first
  capture was read at the entry of the next lot update, not at the end
  of tick 1, and had 0 there. **Unverified:** that the open dialog sets
  it and something outside the lot update clears it.

Known gaps in the capture: the animator pointers were not recorded for
updates 1 to 45. For ticks 1 to 5 the comparison uses the pointers
recorded at update 46, on the same pages.

What would reopen this: a capture whose load does not match ours from
the install's files, or a need to observe something the GDB stub
cannot reach.

## 2026-11-14: Find Location For is ported, and ticks 2 to 5 match (K = 5)

Find Location For (primitive 16) is ported for the modes the Goth house
reaches. With it, tick 2 runs to its end, and ticks 2 to 5 run. Our
state after each of ticks 1 to 5 equals a private capture of the
original after the same tick, in everything compared, the C runtime's
random stream aside. K is now 5. Tick 6 is beyond the capture and stops
loudly.

Some terms. The *place test* is the check whether an object may stand
at a position (virtual function 0x54, 0x433311). The *search* is the
lot's walk over tiles in a square spiral out from a start tile, looking
for one that passes its tests and the place test (0x4347f0). The *C
runtime stream* is the random number generator of the C library
(`rand`), apart from the game's own two streams.

What was decided:

- **Find Location For** (0x44b3c8, `primitives/find_location.rs`).
  Mode 0, and modes 6 and up, which the original treats the same: the
  search starts from tile (1, 1) on level 1, or, with flag 1, from the
  object a local names, and looks first in the start tile's
  room, then anywhere. Flag 2 allows a tile that already holds an
  object; flag 4 refuses tiles with ground flag 0x20. The tile found is
  tested once more, the object is put there with the lot told (0x433340)
  and the true branch is taken. Modes 1 to 5 fail loudly; mode 5 is the
  one that draws random numbers. Native errors 0x15, 0x33 and 0x17 are
  kept.
- **The search and the place test** (`placement/find.rs`), written from
  the original's code: the spiral (0x434ba9), the search's own tile tests
  (a diagonal wall, an empty grid cell, the same room, a pool tile, the
  ground flag), and the place test through 0x4329c1: the border and
  level, the walls the object needs or cannot stand against (0x476f5a,
  only for an object with no wall rules), water, pool, slope, floor,
  tile flag 4, the objects on the tile and the eight around it (0x4337f6)
  and the walls the footprint would cross (0x476dbc). A person, a tile of
  a group, a container, the upper level's floor test (0x4330b7), wall
  rules and two objects that are both not solid fail loudly. The old
  prototype's ports (legacy/crates/sims-sim/src/placement.rs `Spiral`,
  `search_tile_allowed`; legacy/crates/sims-sim/src/lot_acceptance.rs)
  were read alongside; they were checked only against the original's
  code run alone, which is weaker evidence than the capture.
- **Clip names ignore case when nothing matches exactly.** The script of
  people 18 and 21 names the clip "...-Rstand"; the animation library
  has only "...-rstand". The library's lookup (0x5a27f2) compares bytes
  exactly, but the capture shows the original playing "...-rstand" in
  tick 3, with the duration and frame count of that clip. Where the
  original ignores case was not found in its code. `Animations::clip`
  takes an exact match first, then the single name that differs only in
  case, and fails when there are two.
- **Sample names lose their directory.** Members of the animation
  archives whose names carry a directory are now stored under the bare
  file name, which is how scripts and the skeleton files name them. Tick
  3 needed this to find its clip's samples.
- **Create New Object Instance mode 8** (0x44e0c8, the jump table at
  0x44e92e, `primitives/create.rs`) is ported for an ordinary object,
  with no flag or only flag 0x20: the object is made where the object a
  local names stands, tested with the place test, removed again if it
  does not fit, and put there. Turning it (flag 0x20, when its facing
  differs) is not ported and fails loudly, as do a new person and an
  object of a group. This is only reached in tick 6, so it is
  **unverified**.
- **Object 15's dialog** in tick 2 is answered with 3, the second
  button, as the capture's events show. `sims boot` needs `--answers 3`
  to get past it.

How it was checked:

- Each of ticks 2 to 5 was compared with the capture's state at the
  return of the same lot update, outside the repository, with a script
  that builds our state from the install and the house file alone. One
  line per thing compared. Equal, after each tick: the tick count; the
  lot's and the game's random streams; the scheduler cursor; the funds;
  all 64 globals; the removal queue; the registration order; the set of
  objects; per object its GUID, data words, temps, attributes, scheduler
  word, last result, frames with their saved primitive state,
  relationships, slot count, position and two marks; per person the 256
  person data words, motives and previous motives bit for bit, the
  running and last actions, the queue and the objects in use; the
  people's animators, in every field we keep that the capture's memory
  pages hold (100 readings after ticks 3 to 5, 72 after tick 2; 23 fall
  on pages the capture lacks); and, read from the neighborhood file the
  capture's game read, the 463 neighborhood records, the inventories and
  registry byte 0x12.
- The tick-1 comparison is unchanged: equal except the C runtime stream
  and registry byte 0x12, which our tick 2 sets and the capture already
  had set after tick 1.
- The comparison of the load with its trace still gives the three known
  differences, all in object 19's frames.
- Synthetic tests cover the spiral's order, wall crossing, the search
  staying in a room, flag 2, a refusal, the native errors, and mode 8's
  success, refusal and loud failures. The gated test runs ticks 1 to 5
  on the Goth house, pins the state after tick 5 and asserts the tick 6
  stop.

What did not match, and why:

- The C runtime stream is 52 draws further on in the capture after every
  tick. Neither side draws from it in ticks 1 to 5: ours stays at the
  same value and so does the capture's. The likely cause is that the
  capture's game drew 52 more numbers before its load; that is
  **unverified**.
- Read from the install's neighborhood file, records 47, 48, 301, 344
  and 351 and record 301's inventory differ after every tick, because
  the install's file differs from the one the capture's game read.
- Registry byte 0x12 differs after tick 1 only (ours 0, the capture 1).
  What sets it before tick 2 in the original is not known.

What is **unverified**:

- Global 0x6e4ad4, which would make the search refuse more tiles, is
  taken to be 0; the capture does not hold it.
- Everything after tick 5, including Create New Object Instance mode 8.
- The paths of Find Location For and the place test that the Goth house
  does not reach. They fail loudly where they are not ported; the
  ported ones beyond what ticks 2 to 5 use are checked only by the
  synthetic tests against our reading of the code.

Tick 6 starts at tick count 88,449. Object 382 asks for a new object of
type 0x44A86A6C, a person, where the pedestrian portal (object 6)
stands. Making and placing a new person is not ported, so the tick
fails there, before anything is made. What would reopen this: a capture
past tick 5, or a difference found in a later tick.

## 2026-11-14: A person's animator is held, and tick 2 runs to Find Location For

Each person now has an animator in the simulation state. Animate Sim
and the animator's tick are ported as far as the Goth house reaches
them. Tick 2 gets past Animate Sim on object 16 and Stop All Sounds, and
stops at Find Location For (primitive 16) on object 364. K stays 1.

Some terms. The *animator* is the object at person +0xadc (vtable
0x671ee4) that plays a person's animations. A *clip* is one named
animation from the game's animation files. A *layer* is one clip
playing on the person's skin, with its own clock. The animator has
three *slots* for layers: the *foreground* (+0xc), the clip a script
plays; the *base* (+0x14), the clip of the person's posture; and the
*carry* (+0x13c), the clip of something carried. An *event* is a number
a clip sends to the script that plays it. *Presentation* is what only
the picture or the sound uses.

What was decided:

- **What is simulation state.** `World::animators`
  (`sims_sim::animator`) keeps, for each person whose animator has
  ticked or played: the animator's clock, its speed and the speed it
  moves towards, its state, the result of its last route, its position,
  direction and destination, the head's clock, the posture and carry
  bookkeeping, whether the base slot holds a layer, Animate Sim's
  progress through a clip's events, the event queue, and the foreground
  layer's clock, speed, phase, duration and event times. Scripts read
  these through Animate Sim (whether a clip is done, which event came),
  and the animator's tick reads them to decide what it writes to the
  person. Field offsets are the original's and are listed in the doc
  comments.
- **What is presentation**, and left out: the posed bones and the skin
  (0x5994c3, 0x5a15bc, 0x5a0ef5), the head's turn past the countdown
  it keeps in person words 42 and 45 (0x590768), the draw state
  (0x434101), the face (0x58faf6), where a carried object is drawn
  (0x590b27 and the tail of 0x58dc72), the base layer's own clock, and
  sound. The evidence is the original's code: none of these routines
  writes anything a script or the scheduler reads, as far as they were
  followed. The base layer's clip sends no events, so its clock decides
  nothing. That nothing the simulation reads depends on these is
  **unverified** beyond what the capture shows below.
- **The animator before the first tick** (`animator::LOADED`) is
  inferred. The capture's first look at an animator is after tick 1.
  Each value is the one tick 1 leaves alone, or the one that tick 1
  turns into what the capture holds (the doc comment gives the reason
  for each). For example, the speed starts at 1.0, because tick 1 raises
  it by at most 0.2 and the capture holds 1.2.
- **Animate Sim** (opcode 44, 0x58950b, through the person's dispatcher
  0x45cbb3): starting a clip on the foreground slot, the event queue,
  numbered events in order, an interrupt ending the clip, and the events
  a finished clip did not send. The clip's name comes from the global
  file's string table 156 (adult) or 157 (age 17 or less), and its
  duration and frame count from the animation files
  (`sims_content::Animations`).
- **The animator's tick** (0x58dc72), run from the person's tick after
  the decay: the clock (50 per tick, 42 when slowed), the carry choice,
  the base layer's bookkeeping, the speed (towards 0.2 times a base
  plus 1.2, up by at most 0.2 per tick), placing a standing animator on
  its person's tile, the foreground layer's clock (0x597c4c), and the
  head's countdown. All arithmetic is in single precision, as the
  original's.
- **Stop All Sounds** (opcode 48, 0x45253a) takes the true branch and
  changes nothing. Its only effect is a command to the sound mixer.
- **Resetting a person whose animator has ticked fails loudly.** The
  reset also resets the animator (0x589220), which is not ported. The
  resets seen so far are all at the load, before any animator ticks.
- **Snapshot format 11.** `World` gained the animators. The pinned
  states in `tests/resize.rs` and the synthetic world's digest changed.
  The tests strip the animators (`format_10_bytes`) and check that the
  rest of the encoding still gives the format-10 pins. The load's state
  has no animators. After tick 1 the rest of the state is the same as
  before, so the tick-1 pins for formats 9 and 10 still hold.

How it was checked. The private capture of the original running ticks 2
to 5 (the entries below) includes memory pages that hold the three
people's animators and their foreground layers. The scratchpad tool
`tick_verify.py` now reads them and compares each field we keep, as
bits for the floats.

Equal to the original:

- After tick 1: every animator field that the capture's pages hold, 58
  of 58 for people 16, 18 and 21. This includes the speed (1.2, bits
  0x3f99999a), the target speed (bits 0x3fb33334), the state, the
  position and the clock. 23 field readings fall on pages the capture
  did not save. The rest of our state after tick 1 is unchanged and
  still equals the capture, as the entry below lists.
- Where tick 2 stops (at object 364, after every lower ID has run):
  72 of 72 readable animator fields, including object 16's foreground
  layer (priority, clock, duration, speed, phase, event times, frame
  count, end mode, and whether it may be interrupted). 354 of the 355
  objects below 364 equal the capture after tick 2 in data words and
  frames, and the three people also in person data and motives. These
  are compared with the capture after the whole tick, so something later
  in the tick could still change them; the capture shows it does not.

Consistent only with the old prototype's isolated comparisons: nothing.
The old prototype's animation clock is marked approximate, so it was
not used as a guide.

Did not match:

- Object 262, which object 364 makes again just before the stop,
  differs in data word 29: ours -5, the capture's 0. Object 364 is about
  to place it with Find Location For, which is where we stop. That the
  placement writes the word is likely and **unverified**.
- Object 364's own frames, since it has not finished its turn.
- Everything else the entries below list as not matching still does
  not.

**Unverified**, with no evidence either way:

- That every motion of a clip binds to a bone of the person's skeleton.
  A clip with motions is taken to bind at least one.
- Which animation files the original's library loads, in what order,
  and whether it ignores case in a clip's name. A clip that two files
  define differently, or that only a name in another case matches,
  fails loudly.
- The base layer's clock, which is not kept.
- The other skin routine (0x5a0ffd). Either routine updates the layers
  once per tick; which one runs is taken to change only the pose.
- Removing a finished layer from the skin (+0x64), which nothing ported
  reaches.
- The head's paths when bit 0 of person word 42 is set; they fail
  loudly.

What fails loudly: Animate Sim on an object that is not a person, on
the base or carry slot, with clip 0, with a name from anywhere but the
global tables, with a missing clip or one with timed events, replacing a
foreground layer, and at the end of a clip (0x592904); the animator's
tick in any state but 0 and 3, when it has to turn or walk, when it
starts a carry or posture clip, and on a layer end mode other than 0 or
a fading layer; and resetting a person whose animator has ticked.

Found wrong in existing code or docs: removing a person does not drop
its entries in `World::person_ticks` or `World::animators`. Nothing
reads them for a removed person, and no person is removed in the
ticks run, so it is noted and left.

What tick 2 needs next: Find Location For (0x44b3c8). Mode 0 searches
for a free place near the reference object (0x4347f0, which draws
random numbers through 0x46f457), tests each place (0x432679,
0x4329c1) and puts the object there. In the capture, object 262 ends
tick 2 at (632, 344, level 1). After that, tick 3 has people 18 and 21
start a clip, and an Animate Sim clip's end (0x592904) will be needed.

## 2026-11-13: The neighborhood's people are held, and tick 2 runs to Animate Sim

The simulation now holds the neighborhood's people the way the original
does while a lot runs. Tick 2 of the Goth house gets past the scripts
that read and write them, and stops at Animate Sim on object 16, which
needs a person's animator. K stays 1.

Some terms. A *record* is the original's 628-byte entry for one person
of the neighborhood, kept in a list in the neighborhood object (+0x148):
an ID, the GUID of the person's type, a revision, 256 words of *person
data*, *relationship rows* (one per other person, a list of numbers
each), a cached friend count and a mark that says the count is out of
date (the *stale* mark). A record's *slot* is its place in that list. A
neighbor's *inventory* is a list of tokens (kind, GUID, count). The
*frame storage* is the block of words in which the original keeps an
object's script frames; a new frame's locals start with whatever an
earlier frame left there.

What was decided:

- **The records live in `World::neighbors`** (`sims_sim::neighbors`):
  every slot, empty ones included, with the inventories and registry
  byte 0x12, which a relationship write sets. `World::read_neighbors`
  builds them before the load, as the reader (0x42feb2) and the
  neighborhood's reload (0x424806) do: the file's records as read, then
  person data word 61 (the family) rebuilt from the families in their
  sorted order (0x425209, 0x425244). Ported from the old prototype's
  `prepare_family_reload` and `rebuild_family_links`
  (legacy/crates/sims-vm/src/neighborhood.rs). A person type with no
  record fails loudly: the reload would make one, which is not ported.
  `sims_content::Neighborhood` now keeps the empty slots, since a script
  names a record by its slot plus one (0x4256f4), and reads the
  inventories from the NGBH chunk (0x4254f6). Constructing a person and
  attaching it to its record now read these records, not a copy in
  `People`.
- **What reads and writes them**, each ported from the original's code
  named in its doc comment: Set To Next modes 5 and 7 (0x425b15,
  0x44ed07); expression scope 32, a record's person data (0x4537e3);
  Relationship with flag 2, and between two people on the lot, which
  keep their rows in the records (0x44d17c; the row's key is the target
  record's own ID; a write marks both records stale and sets the
  registry byte, 0x44485b); Manage Inventory operation 3 (0x4552f8).
- **Idle For Input** (opcode 17, 0x45db94), through the person's own
  dispatcher (0x45cbb3), with the test of whether something interrupts
  the wait (0x45d7c0: a queued action of higher priority or pushed to the
  front, or a very low motive on a residential lot). Checked against the
  old prototype's `idle_for_input` and `should_interrupt`
  (legacy/crates/sims-vm/src/action.rs). Starting the queued action is
  not ported and fails loudly.
- **Get Distance To** (opcode 11, 0x451f27, distance 0x434148): the sum
  of squares in 32-bit integers, then single precision for the rest, and
  truncation toward zero. The prototype has no version of it.
- **A returning frame leaves its return target in its node word.** The
  interpreter wrote the target of every branch into the frame's node
  word, except a return. The original writes it for a return too (0xfe or
  0xff, at 0x474cea and 0x474d1b) before it pops the frame, so the frame
  storage keeps it. This was wrong in the existing code. The evidence: in
  the capture's tick 2, a person's new frame finds 254 in a local that
  lands on such a word; ours found the old node, 15.
- **Snapshot format 10.** `World` gained the records. The pinned states
  in `tests/resize.rs` and the synthetic world's digest changed, for two
  reasons. The records: the tests strip them (`format_9_bytes`) and check
  the remaining encoding. The return targets: before the pins changed,
  every pinned state was dumped under the old and the new interpreter
  and compared outside the repository. Everything but the frame storage
  was equal. In the frame storage, every word that differed sits at a
  frame's node word and now holds 0xfe or 0xff. The tick-1 gated test
  still equals the capture.

How it was checked. The private capture of the original running ticks 2
to 5 (the one in the entry below) holds the 463 records, the
inventories and the registry byte after every tick. The scratchpad tool
`tick_verify.py` now compares them too, one line per field.

The capture's game read another neighborhood file. Its records equal
ours read from the emulator guests' `Neighborhood.iff` (private, outside
the repository). The
install's file differs from that one in five records: 47 and 48
(revision, person data, relationship rows of three numbers instead of
two), 301 (revision, person data, and an inventory), 344 and 351
(person data). So the comparison is run both ways: with the guests' file
every record field and the inventories are equal after tick 1; with the
install's file only those five records and the inventory differ.
Everything else in our state, after tick 1 and where tick 2 stops, is
the same from either file. The gated tests use the install's file and
say so.

Equal to the original:

- After tick 1, all 463 records (ID, GUID, revision, person data,
  relationship rows, friend count, stale mark) and the inventories, read
  from the same file. Before this, the load trace had shown family word
  61 of every record equal to ours.
- Where tick 2 stops: object 16's frames (trees, nodes, stack objects,
  arguments, locals) equal the capture's after tick 2. The capture has
  object 16 at that same node after ticks 3, 4 and 5 as well.
- Part way through tick 2, at that stop, records 32, 33 and 34 equal the
  capture's after tick 2 except two rows (33 to 34, and 34 to 33), which
  in the capture gain a third number later in the tick. That is a
  spike's comparison of a state taken part way through the tick, not a
  like-for-like one, so it is not counted.

Consistent only with the old prototype's isolated comparisons: the
family rebuild and Idle For Input's paths, which the prototype has in
the same form.

Did not match: registry byte 0x12 is 1 in the capture after tick 1 and 0
here. A load trace of another run shows it 0 at the resize and 1 after
the object read; what sets it there is not known, and nothing ported
reads it. The C runtime's random stream is still 52 draws further on in
the capture (the entry below); ours does not move in tick 2 up to the
stop.
The six type attribute words no file holds (the entry two below) play no
part in these results: the comparison is the same with and without them.

**Unverified**, with no evidence either way:

- Get Distance To's value. It is ported from the code alone; the
  comparisons that cover it are whole ticks, and tick 2 does not finish.
- That the original keeps nothing else in the records that a lot
  changes. The capture's records change only in the rows named above
  through tick 5.
- The family friend count a relationship write also marks (0x4246b9,
  family +0x124). The families are not kept; nothing ported reads it.
- Manage Inventory's token layout (12 bytes: kind, GUID, count), read
  from the code; every inventory in the capture is empty.
- The interrupt test's paths: the capture does not show which way it
  went for the Goth house's people.

What fails loudly: Animate Sim (opcode 44), whose animator tick
(0x58dc72) moves the person and runs the animation clock; Idle For
Input starting a queued action (0x45dc14); Manage Inventory operations
other than 3; a person type with no record; a record whose GUID has no
definition, in Set To Next mode 7 and Manage Inventory; Set To Next mode
11; an empty slot in the file's inventory list; and everything the entry
below lists.

What tick 2 needs next: Animate Sim and enough of the person's animator
to say, each tick, which events an animation reached and whether it is
done, from the original's code (0x58950b, 0x58dc72). The old prototype's
animation clock is marked approximate there, so it is no guide. With it
tick 2 should reach what the capture shows next: object 15's two-button
dialog, answered with the second button, and object 262 made again.

## 2026-11-13: Ticks run, and the first one matches the original

`sims boot <install> <house> --ticks N` now runs N ticks after the load
and prints the hash. On the Goth house tick 1 runs and our state after
it equals the original's. Tick 2 stops at something not ported, and the
boot fails and names it and the tick.

Some terms. A *tick* is one lot update, the original's 0x46d345. The
*scheduler* is the part of it that goes over the objects. An object's
*countdown* is the low half of its scheduler word: while it is above 0
the scheduler counts it down instead of running the object. A *dialog
record* is what the Dialog primitive leaves for the game to show; its
*status* says whether it was shown and which button answered it. A
*modal* dialog stops the game until it is answered.

What was decided:

- **The lot update, the scheduler pass, the object tick and the person
  tick are ported** as `Interpreter::lot_update`, `scheduler_pass`,
  `tick_object` and `tick_person` (`crates/sims-vm/src/tick.rs`), read
  from 0x46d345, 0x46d553, 0x4521f7 and 0x45cc8c.
- **The end of the lot update is more than the removals.** The roadmap
  said 0x4412d1 carries out the queued removals. It first goes over the
  dialog records, showing new ones and destroying finished ones, then the
  game's special panels, and only then, after a full pass, the removals;
  it ends with a check that a household Sim is selected. Ported as
  `Interpreter::end_of_update`, without the panels, which no ported
  primitive opens.
- **Dialog is ported in the form the simulation needs**
  (`primitives/dialog.rs`, 0x451552): records in `World::dialogs`, the
  statuses 0 to 5, the true branch for the first button and the false
  branch for the second and third. What a dialog says and shows is not
  modeled. Kinds 3 (text) and 5 to 15 (the special panels) fail loudly.
- **The player answers modal dialogs inside the end of the update.** The
  original waits there, in the dialog's own loop, until the dialog is
  answered. `sims_vm::Player` gives the answer. `sims boot` answers a
  one-button dialog with its only button, and a dialog with a choice
  with the next value of `--answers`; with none left the tick fails. A
  dialog that does not stop the game fails loudly.
- **Motives** (`sims_sim::motives`): the decay (0x420fb6), the mood
  (0x45d165) and the motive changes (0x45cdb3), all in single precision.
  The tuning comes from the global file (`Program::motive_tuning`).
- **Snapshot format 9.** `World` gained the dialog records with the
  registry's word 0x100, and each person's tick state: the tick of the
  last decay (person field 0x810) and the room last seen in (0xafc).
  Before a tick these encode as three zero bytes at the end. The gated
  tests check every state pinned under format 8 by dropping those three
  bytes (`format_8_bytes` in `tests/resize.rs`), and the synthetic
  world's old digest the same way.

How it was checked. A second run of the original on the same install and
house is private and outside the repository. Its observer attached while
the game sat in the dialog loop of its first tick. It holds the full
state at the entry of the next lot update, which is the state after tick
1, and the states after ticks 2 to 5, with a log of the dialog functions
it passed. The comparison is the scratchpad tool `tick_verify.py`.

Equal to the original after tick 1: the tick count, the lot's and the
game's random streams, the scheduler cursor, the funds, all 64 globals,
the removal queue (empty), the registration order, the set of objects
(the 7 IDs the load queued and 2 the tick queued are gone, in both), and
for each of the 377 objects its GUID, 72 data words, temps, attributes,
scheduler word, last result, frames with their saved primitive state,
relationships, slot count, position, and the removal and selected marks;
for each of the three people its 256 person data words, its 16 motives
and 16 previous motives bit for bit, its running and last actions, its
queue and its number of objects in use. Of the 31 lines compared, 30 are
equal. The original's log agrees with the dialogs: in tick 2 the five
controllers acknowledge an answered record and go on, and in tick 3 object
15 acknowledges a record answered with the second button.

Did not match: the C runtime's random stream. The capture's state is 52
draws further on than ours, after tick 1 and after every later tick (it
never moves in that run's ticks 2 to 5). Our tick 1 draws none. The trace
of the load, from another run, equals ours at the start of tick 1. The
likely cause is that this run's game drew 52 more numbers before its
load; that is **unverified**, and so is that the original's tick 1 draws
none.

Consistent only with the old prototype's isolated comparisons: the mood
curves' parsing (legacy `Curve::parse`, compared there with 0x461423 and
0x46137b run alone). The decay and the mood themselves are equal to the
original (above), for the Goth house's people at this one decay.

**Unverified**, with no evidence either way:

- What an object's animated sprites, a person's animator and the graphics
  work every tenth tick do; they are taken as presentation and not
  modeled. The same for signal 0xdb (only the interface takes it) and
  signal 0xf7 (only the house viewer); the lot's second subscriber, at lot
  0x10, which sees every signal first, is not known.
- The instruction limit of one tick, 458,605 (read from a memory image).
- That making a dialog record runs no object code.
- Kind 2 (three-button) dialogs, an informing dialog, and the second and
  third buttons' effect on temps and registry word 0x100: read from the
  code, not exercised by the capture.
- The scheduler pass stops at the highest ID held; the original's table
  can be longer. It shows only for an object made during a pass with a
  higher ID.

What fails loudly: the hour's rollover; a scheduler tick that returns
false; a thread that returns from its outermost frame or runs past the
limit; a person entering or leaving an indoor room; the lot service every
tenth tick (0x4165e3; first at tick count 88,450 on the Goth house);
person word 1; removing a tile of a multi-tile object or an object that
owns a dialog at the end of the update; no selected Sim; dialog kinds not
ported; a choice with no answer.

What tick 2 needs: Set To Next mode 5, over the neighborhood's people.
Object 16, a person, reaches it in a tree of the people's semi-global
file that also reads those people's relationships. Porting it means
holding the neighborhood's people (their records, person data and
relationships) in the simulation. That would reopen this entry's K = 1.

## 2026-11-13: The load runs to the first tick, and rooms have light and scores

`sims boot` now runs all 22 load steps on the Goth house. The state it
ends in is the one the original's first tick starts from. Ticks are not
implemented.

Some terms. The *selected Sim* is the one the player controls; the
original marks it with a flag on the object and writes its ID to global
3. An *indicator* is one of two objects the game keeps on every lot; the
first is the marker over the selected Sim. A room is *marked* when it
waits to be counted. A level *waits* when the lot knows that one of its
rooms is marked. The *room update* counts the marked rooms.

What was decided:

- **Step 16 clears global 3** (`Interpreter::announce_house`, 0x44302e).
  When the house is announced and no Sim is selected, the registry asks
  for the indicators and then announces that nobody is selected, which
  clears global 3. The roadmap had this as part of step 17.
- **Which Sim is selected** (`Interpreter::select_resident`, the end of
  0x4f1ae9). A selected Sim that belongs to a family stays. Otherwise it
  is the family's first member: the first object of that member's type.
  Whether it may be selected depends on the lot's zoning and number and
  on the Sim's visitor word (0x4e989b). The roadmap had asked whether it
  is the Sim the house saved in global 3 or the family's first member. It
  is the first member: global 3 is cleared before the choice, and no
  save holds the Sim's mark.
- **Selecting a Sim** (`Interpreter::select`, 0x445449): the mark
  (`Object::selected`), the indicators (0x44664c), then global 3.
- **An indicator that leaves the lot is taken off twice.** When an
  object leaves the lot the registry is told (0x447d2b), and it takes an
  indicator off once more (0x4481eb). So the indicator is asked function
  10 twice. `placement.rs` used to say that telling the registry changes
  nothing the simulation holds.
- **Step 19 is `Interpreter::go_live`**: every room is marked, the
  lot's statistics are worked out, and global 19 becomes 0. Of the
  statistics (0x416085) only globals 23 and 37 are ported.
- **Steps 21 and 22 are `Interpreter::before_frame`**: the rooms that
  wait are counted, and global 12 becomes 0.
- **The room update is ported whole** (`World::update_rooms`, 0x466c3c):
  the objects standing in each room, the room's light and score, the
  lot's outdoor score, and the rule that a room whose light changed is
  marked again. `Rooms` now holds which levels wait.
- **Step 10a is two updates.** The original counts the rooms after step
  10, then marks them all when the house read ends (signal 0xdd) and
  counts what waits. Before, one count gave the same sizes. With light
  it does not give the same marks: the first count changes every room's
  light, which marks every room again.
- **The room arithmetic rounds every operation to single precision**
  (`Ext::single` in `sims-core`). See below.
- **The snapshot format is 8.** New: each object's selected mark, each
  room's counts, light, score and pool mark, and the rooms' waiting
  levels, outdoor sum and outdoor score.

How it was checked. The private trace of the original loading the Goth
house stops at the entry and return of the funds setter, of the
selection (twice), of the mode change (twice) and of the lot's part in
it, at the entry and return of the wait check, and at the entry of the
first tick. It holds every call of the function that runs an object's
callback (0x422d46). Our run was compared with it outside the repository
(`live_verify.py` in the private scratchpad, one line for each thing
compared).

*Equal to the trace:*

- **The whole state at seven stops**: the entry of step 11 (after the
  room count), the entry of the funds setter (after step 16), the return
  of the selection (after step 18), the return of the lot's part of the
  mode change and the entry of the second selection (inside and after
  step 19), the entry of the wait check (after step 21), and the entry
  of the first tick. At each: everything the entry below lists for the
  end of step 13, and each object's selected mark.
- **The rooms at each of those stops**, all 11: the twelve counts, the
  lights and people, the score and the light bit for bit, whether the
  room is outdoors, whether it is a pool, the mark, the levels that
  wait, and the lot's outdoor sum and score bit for bit.
- **The callbacks of step 18**, in order: 383:10, 383:10, 383:9, 21:9.
  No other callback is asked between the end of step 13 and the first
  tick, there or here.
- **Steps 13 to 15** as before; the comparison of the entry below was
  run again with the room fields added.

Why the six indoor rooms stay marked after step 21, which the roadmap
could not explain: their light changed. At step 10a every indoor room
was fully lit through its windows. Step 13 reset the windows, which
cleared the light each lets in. So at step 21 the indoor rooms come out
dark, or lit by their lamps only, and a room whose light changed is
marked again. The outdoor rooms have the daylight both times.

The original's FPU and single precision. One room's score in the trace
is 3 units in the last place away from what `sims-core`'s 64-bit
register model gives. It is exactly what the sum gives when every
addition is rounded to 24 bits. The other ten rooms are equal either
way. So the FPU's precision control is set to single precision when the
room update runs, as Direct3D sets it. The room code now rounds each
operation that way, and all 11 scores, both times, are equal bit for
bit. Motive values compared earlier were equal under the 64-bit model.
The sums that made them may not tell the two apart. That has to be
looked at again when motives change in a tick.

The second 383:10. The trace shows function 10 asked of the indicator
twice, and the code that moves the indicators asks once. To find the
second caller, the original's selection code (0x445449) was run in an
emulator on the memory the trace holds at its entry. It ran through both
calls and showed the chain: remove from lot (0x433125), told to the
registry (0x447d2b), which removes an indicator again (0x4481eb). The
emulation is weaker evidence than the trace, and only the caller rests
on it. The order of the four callbacks is equal to the trace.

*Consistent with the old prototype's isolated comparisons only:*

- The room update on a lot with a pool, and an object's share when it
  is hidden, has a negative room word, or is one of the two categories.
  The prototype compared the score arithmetic and the object visit with
  the original's code run alone. The Goth house has no pool.
- The names and defaults of the room constants. The Complete Collection
  has the constants the code defaults to.
- The zoning rules of 0x4e989b for other lots than a residential one.
  They were read from the code here; the prototype read the same.

What changed in earlier work, and why:

- **`World::order` and `World::scheduler_cursor` were documented
  wrongly.** The comments said the scheduler visits the registration
  list and that the cursor is a place in it. The scheduler's pass
  (0x46d553) goes through the registry's table of objects by ID, the
  table the ID lookup (0x40743b) reads, and the cursor is an ID. Only
  the comments changed; nothing uses either yet.
- **`World::update_rooms` takes the constants, the level and the
  objects on a tile.** `sims-vm` supplies them
  (`Interpreter::update_rooms`). The room tests in `sims-sim` count with
  no objects.
- **The gated tests pin new digests**, because the encoding changed.
  Each state that was pinned under format 7 is also encoded without the
  new fields (`format_7_bytes` in the test file), and that encoding
  still has the digest pinned before. So the states are the same ones.
  The chain back to formats 6, 5 and 4 now starts from that encoding.
- **The gated test of step 13 no longer says steps 14 to 16 change
  nothing.** Step 16 clears global 3. A new gated test takes the load
  from there to the first tick.
- **The roadmap's step 17 was "clear the selected Sim and set the funds
  again".** The clearing is step 16's. Step 17 is the funds.

What is **unverified**:

- **Selection on any lot but this one**: a family whose first member is
  not on the lot or may not be selected, a visitor, a community lot.
- **Three things the interface holds**, taken as the trace shows them:
  every family member is on the list of portraits; the switch that lets
  any Sim be selected (0x6e4f64) is off; a person's slot 1 takes an
  indicator. The trace's memory shows that slot flag on the two Goths
  whose slots it holds.
- **The order of the original's list of people**, which decides who is
  selected if two people have the mark. The registration order is used.
- **The statistics pass beyond globals 23 and 37**: global 22, the
  value of the lot's objects and architecture, two flags of the family's
  record. They are left as they are. In the trace they do not change,
  but they already hold what the house saved.
- **The bits of global 37 other than bit 0.** The Goth lot sets only
  that one. The others are read from the code.
- **The daylight's multiplier** (0x6f015c), which is 1.0 in the trace's
  memory and is taken to be 1.0 always.
- **The level the first room update of step 10a counts.** It is taken
  to be every level. The second update counts every level whatever the
  first did.
- **The time of day other than day**, and the lot's outdoor score at
  night, dawn and dusk. The trace is by day.
- **Single precision outside the room update.** See above.

What is not modeled: the indicator's and the Sim's graphics during the
selection; the star markers of the Superstar pack, which the original
handles only for a Sim that is not selected; the lights the room update
places in a room and the lighting of the tiles; the rooms' cached route
rectangles; what a person is told when the mode changes; the count of
the family's friends; and a field of the family's record that the end
of the load clears.

What fails loudly: deselecting a Sim; selecting a Sim with a date, or
one whose slot 1 holds something, or one of several tiles; a family
member's type that two definitions name; a star marker leaving the lot;
an indoor room that is dark, has no lights and has people in it, where
the light it shows depends on the room knowing it is occupied
(0x468802), which is not ported; and `sims boot --ticks` with a number
above 0.

About the first tick. It is not implemented, because one tick could not
be both built and checked in this piece of work. The roadmap now says
what a tick is in the original (0x46d345 and 0x46d553, read from the
code), what the first one needs, and what there is to check it against.
The main finding: an earlier capture of another run of the original, at
tick 88,445, differs from our baseline only in what one tick would
change, in every object field compared, and has the same two game
random streams. So the first five ticks can be checked against it.

What would reopen this: a house with a pool, or loaded at night; a lot
whose people stand indoors in the dark; a house where the selection is
not the family's first member; a trace that shows the statistics pass
writing a value; any float result that single precision does not give.

## 2026-11-13: People are visited like the original visits them, and the load runs until the interface takes over

Load step 13 visits every object on the lot. It used to stop at the first
person. `sims boot` now visits people too, finishes the step, and runs
steps 14 to 16. On the Goth house all 382 visits run. The boot stops
after step 16, before the original's interface selects a Sim.

Some terms. A person's *neighborhood record* is its entry in the
neighborhood file: an ID, the person's type, and 256 person data words.
A record has a *revision*, which says how many of those words it keeps
for the person. A type has *changed* when the scripts installed now
differ from the ones the house was saved with. Function 3 of an object is
its Reset tree and function 0 its Init tree.

What was decided:

- **A person's visit is `Interpreter::visit` too** (0x459bf6). It differs
  from an object's in three ways. The person first takes the fields its
  neighborhood record keeps (`attach_neighbor`, 0x4258a3). It is not
  asked its Load function. And if its type changed it resets in its own
  way (`reset_person`, 0x45e791).
- **The record's fields.** The record's ID goes into person data word 31.
  Then the words `persistent_fields` lists for the record's revision are
  copied from the record: 29 at revision 1, 48 at revision 10. The record
  is the first of the person's type in the file.
- **A person's reset**, in this order: person data word 30 is cleared;
  the gender, age and skin tone are read again from the body strings of
  the person's type (`reload_profile`); the thread restarts at Main and
  then the Reset tree runs on top of it; person data words 0, 40, 59, 33,
  71, 72 and 73 and data words 34 and 9 are cleared, with any motive
  changes in progress; and each queued action is dropped, its target
  being asked function 4. An object's reset runs the Reset tree first and
  restarts the thread afterwards. A person's does it the other way round.
- **Only a person that is doing nothing can be reset.** A person that is
  using an object, has a running action with a target, is in the middle
  of a route, is inside another object or off the lot, or has person data
  word 73 set fails. The original does more for each. No person of the
  Goth house is in any of those states.
- **The cached room and the motive curves are not kept.** After the
  visit the original notes the person's room, which it uses to light the
  room, and loads the curves that turn each motive into its share of the
  mood. The lighting is not modeled. Nothing reads the curves until a
  person's motives are updated in a tick. `curve_table` says which string
  table the curves come from. So `World` did not change, and the snapshot
  format is still 7.
- **`People` now carries each person type's profile**: three of its body
  strings (gender, age, skin tone) and the skin tone names from its
  semi-global file.
- **Step 14 is `Interpreter::ensure_indicators`** (0x445e4a): create the
  two indicators unless the lot has them. Step 16 calls it again.
- **Step 15 is `World::finish_selection`.** After the visits the function
  that selected the household may delete people who do not belong, create
  a tutorial controller, reset the lot, and work out who is a visitor.
  None of that is ported. The step checks that none of it could do
  anything, and then changes nothing: every person is of the selected
  family and not a visitor, global 26 is 0, and the family's record does
  not ask for a reset (flag 4). Otherwise it fails and says which.
- **The roadmap's step list was wrong at its end and is corrected.** It
  had one last step, "create the indicator objects and finish
  selection". The trace shows nine more stops after the visits. Steps 14
  to 16 belong to the loader and are ported. Steps 17 to 22 are the
  interface's: it clears and sets the selected Sim, moves an indicator
  into that Sim's slot, goes to live mode, and updates the rooms. They
  are written down in the roadmap from the trace and are not ported.

How it was checked. The private trace of the original loading the Goth
house holds a full state at the return of step 13, and at three stops
after it: the return of the indicators' creation after the last visit,
the return of the lighting update that ends the household selection, and
the return of the indicators' creation called when the house is
announced. For every visit it holds each call of the function that runs
an object's callback (0x422d46). Our run was compared with it outside
the repository (`people_verify.py` in the private scratchpad, which
prints one line for each thing compared).

*Equal to the trace:*

- **Every call of every visit.** For each of the 382 visits: the object
  visited, and the calls it makes, as object and function, in order.
  1,781 calls. The three people are visits 202 to 204, objects 21, 18
  and 16, and each is asked function 3 and then function 0.
- **The whole state at the return of step 13.** All 386 objects: GUID,
  the 72 data words, temps, attributes, scheduler word, last result,
  every frame's tree, node, stack object, parameters, locals and saved
  primitive state, the relationships, the number of slots, the position
  and the removal-without-resets mark. For the three people also the 256
  person data words, the 16 motives and the 16 previous motives bit for
  bit, the running action and the last action (ten fields each), the
  queued actions and the number of objects in use. The tick, the 64
  globals, both game random streams and the C runtime's, the funds, the
  removal queue, the indicator references, the registration order, the
  instance lists, which rooms wait to be counted (8 of 11), and the
  arrays inside the lot's size, the object grids included.
- **The same at the three later stops**, after our steps 14, 15 and 16.
  The trace changes nothing compared between them, and neither do we.
- **The two stops in the middle of visit 140**, as in the entry below,
  now with the people's motives and actions compared as well.

This is the first check of what is not an object field after visit 140.
The entry below called that unverified: the global random stream, the
globals, the arrays and object grids, the removal queue and the rooms
waiting to be counted. All are equal at the step's return. The global
random stream moves during the step, so its being equal is not empty.

What the trace shows of a person's visit, and what it cannot show:

- It shows the record's fields being taken: person data words 46 to 55
  change, to the record's values. The other words copied already hold
  the record's values.
- It shows the thread restarting: three frames become one Main frame at
  node 0 with no stack object.
- It shows what the Reset and Init trees do: the last result becomes
  true and seven motives change. Ours are equal bit for bit.
- It cannot show the words the reset clears or reloads being written.
  In this house they already hold the values written: the cleared words
  are 0, and the gender, age and skin tone read from the body strings
  are the saved ones. So the trace shows them unchanged, and so do we.
- It cannot show whether Main is restarted before or after the Reset
  tree runs. The result is the same here.

*Consistent with the old prototype's isolated comparisons only:*

- The order inside a person's reset, and which words it clears. The
  prototype ran the original's reset alone on 16 cases.
- The list of fields a record keeps, by revision. The prototype compared
  126 cases. The Goth records have revision 10.
- The body strings read by the reset and how they become numbers. The
  prototype compared 216 cases.
- The conditions of step 15, which the prototype compared on 192 cases.
- A queued action being dropped and its target asked function 4. No
  person of this house has a queued action.

What did not match, and why it does not matter. Object 19's second frame
has tree ID 0x8118 in the trace and 0x118 here and in the house file, at
every stop before its visit. Its visit is number 206. The visit makes
the same three calls here as in the trace (functions 2, 3 and 0), and at
the step's return object 19 is equal in every field compared: its reset
replaced the thread. So the bit does not change what the visit does.
What sets it is still not known. Six type attribute words belong to the
game that was running; with those copied from the trace's start they are
equal.

What changed in earlier work, and why:

- **The gated test of step 13 now runs the whole step.** It still shows
  the state pinned after visit 201: it makes the first 201 visits one at
  a time on a copy and gets the old digest. No other pinned digest
  changed, because `World`'s encoding did not.
- **`sims boot` no longer says it stops before the indicators.** It
  reports steps 14 to 16 and stops before step 17.
- **The roadmap's "Progress" said the boot runs through step 12.** It had
  been running into step 13 since the entry below.

What is **unverified**:

- **A person that is doing something**, as listed above. It fails.
- **A neighborhood record of a revision below 1**, which holds no person
  data, and a person with no record. Both fail.
- **The motive changes in progress being cleared.** The trace does not
  hold them, and the Goths have none.
- **Which motive curves a person gets** (`curve_table`). Nothing loads
  them yet. The trace holds the two global tables at the end of the
  load, not which person uses which.
- **Step 15 on any other lot**: a visitor, a person of no family, the
  tutorial house, a family that asks for a reset. All fail.
- **Steps 17 to 22**, which are not ported. What the trace shows of them
  is in the roadmap. Two things about them are not known from the
  trace: how the game picks the Sim it selects, and what makes the six
  indoor rooms still wait to be counted after the room update.

What is not modeled: a person's animations, graphics, action icons and
accessories during a reset; the guard against a reset inside a reset;
the cached room and the lighting it drives; the lighting update at the
end of step 15; and the signal that announces the house, beyond the
indicators it asks for.

What fails loudly: resetting a person in any of the states above; a
person type with no profile or no neighborhood record; a record with no
person data; a body string age that overflows 32 bits; and each
condition of step 15.

What would reopen this: a house whose people are in the middle of
something when saved; a house with visitors; a capture that holds a
person's motive changes or shows which curves it uses; anything that
shows a reset writing a value that differs from what the trees or the
save left.

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
