# Ticks run, and the first one matches the original

*Decided 2026-11-13.*

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
