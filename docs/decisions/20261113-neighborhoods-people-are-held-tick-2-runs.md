# The neighborhood's people are held, and tick 2 runs to Animate Sim

*Decided 2026-11-13.*

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
to 5 (the one in [20261113-ticks-run-first-one-matches-original](20261113-ticks-run-first-one-matches-original.md)) holds the 463 records, the
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
the capture ([20261113-ticks-run-first-one-matches-original](20261113-ticks-run-first-one-matches-original.md)); ours does not move in tick 2 up to the
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
