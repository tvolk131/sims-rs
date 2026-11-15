# A script can make a person, and ticks 6 to 13 match (K = 13)

*Decided 2026-11-14.*

In tick 6 of the Goth house, object 382 makes a new person (Create New
Object Instance mode 8). That is now ported, and so is what the next
seven ticks reach. Our state after each of ticks 1 to 13 equals the
capture of the original. K, the number of ticks after each of which our
state equals the capture's, goes from 5 to 13. Tick 14 stops loudly at
routing.

Some terms. A *capture* is the private record the harness took of the
original running the Goth house: its full state after each of lot
updates 1 to 100, and one line per lot update in `updates.jsonl` with
the tick count, the three random streams and the scheduler cursor. An
*object use* is an entry in a person's list of the objects it is using
(person +0xb00). The *usage counts* are six pairs of numbers in the
house file's model (HOUS 1): for each of the last five days and today,
how often a household member was counted, and how often while routing.

What was decided:

- **The person factory is ported** (`crates/sims-vm/src/person_factory.rs`,
  `Interpreter::create_person`), following the person path of 0x440b2d
  from 0x440d63: a guard against pets, the lowest unused ID, the
  storage, the skeleton (which draws from the C runtime's stream), the
  neighborhood record, default traits if the interests are low, the
  construction reset (0x4598e8: Init twice, the first update once, the
  animator reset), and the price and ready flag. Ported from the old
  prototype's `PostConstruction`, `ConstructionReset`, `InitialReset`,
  `default_traits` and `attach_person_neighbor`
  (legacy/crates/sims-vm/src/person_factory.rs).
- **The pet guard fails loudly.** A person type whose skeleton is `dog`
  or `kat` is made only if a field of the running game (+0x66 of the
  object at 0x6e48bc) has bit 0x20 or 0x80 (0x440d8b to 0x440dab). That
  field is not modeled.
- **Mode 8 makes a person** (`primitives/create.rs`): the factory, then
  the record's interests and personality (0x425930), then, with flag
  0x20, the stack object's facing whole (0x44e684). A new person whose
  person word 84 or 68 has bit 0 set, or word 74 bit 0 or 1, goes
  through 0x45810c, which fails loudly.
- **The place test handles a person** (`placement/find.rs`): a person
  skips the wall rules (0x432ac5); one with person word 68 bit 0 collides
  with nothing; another object is in its way as 0x433975 says (when not
  solid, with data word 8 flag 2; when solid, without flag 0x10). Person
  word 84 bit 0 fails loudly (0x43383f). What the interface does when a
  person comes onto the lot (0x44752e) fails loudly in three cases (the
  selected Sim, person word 77 above 0, person word 81 of 10 or more)
  and otherwise does nothing.
- **The house's usage counts are state** (`World::usage`,
  `HouseUsage`). They are read from the house file's model with the
  simulator state (0x416568; `World::read_house_model`). Every tenth
  tick, each awake household member adds one to today's total, and one
  to today's routing count if it is in the middle of a route (0x45ce14
  to 0x45ce5c, 0x4165e3). The roadmap said before that the person asks
  the lot something there; that was wrong.
- **Object uses are counted** (`crates/sims-vm/src/object_uses.rs`).
  When a person runs a tree of another object with Run Tree By Name in
  mode 2, the person's virtual function 0x34 (0x45e59b) adds an entry
  and the object's data word 62 goes up. After each frame a person's
  thread pops, its virtual function 8 (0x45e5fd) removes the last entry
  if its frame is gone. When the list empties, the running action ends
  (0x45e6cd). The result of a return is written after that (0x474da0),
  so the ended action's flag 0x20 takes the result before the return.
  Ported from the prototype's `push_target_tree_args`,
  `pop_person_reset_frame` and `clear_person_action`.
- **The first clip reads the appearance again.** A new animator has its
  flag +0xd8 set (`Animator::profile_stale`); looking up the first clip
  reads the person's appearance and clears it (0x5893b3).
- **Snapshot format 12** adds that flag at the end of each animator and
  the usage counts at the end of the encoding. Every older pin in
  `tests/resize.rs` and `sims-sim/tests/world.rs` still holds once the
  new fields are taken out (`format_11_bytes`, `without_usage`), so the
  pinned states are the same.

How it was checked: our state after each of ticks 1 to 13 was compared
with the capture's after the same lot update, field by field, floats
bit for bit. The fields are listed in the doc comment of
`the_goth_house_first_tick_matches_the_original`: the `updates.jsonl`
line; the tick count, streams, cursor, funds, globals, removal queue,
registration order and set of objects; the 463 neighborhood records,
the inventories and registry byte 0x12; every object's GUID, data
words, temps, attributes, scheduler word, last result, frames,
relationships, slots, position and marks; every person's data, motives,
actions, queue and objects in use; and the people's animators. 617
lines are equal. Ticks 1 to 5 were also compared again with the first
capture, and the load with its trace; both give the same differences as
before.

What did not match, and why:

- After tick 1, global 12 (17 in the original, 0 here) and registry byte
  0x12 (set in the original from its load, here from tick 2). These are
  the known differences of [20261114-comparison-harness-runs-again-capture-holds-1000](20261114-comparison-harness-runs-again-capture-holds-1000.md).
- Tick 14 does not finish. Person 259 reaches Go To Routing Slot
  (primitive 45, 0x45abce), the start of routing, which is not ported.
  Where it stops, person 259's frames and objects in use and the 258
  objects before it in the tick equal the capture's after tick 14.

What is **unverified**:

- The usage counts. The capture does not hold them, and nothing ported
  reads them.
- The end of an object use and of a running action. No use ends in the
  ticks that run.
- One local of person 259's third frame at the tick 14 stop. The
  original left half of a pointer there (2623); we hold it as unknown,
  and it is not compared.
- The parts of the person factory, mode 8 and the place test that the
  Goth house does not reach. The synthetic tests check them against our
  reading of the code.

What would reopen this: a difference found after tick 13, or a capture
that holds the usage counts.
