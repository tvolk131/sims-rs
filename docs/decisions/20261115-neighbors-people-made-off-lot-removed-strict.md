# Neighbors' people made off the lot and removed; strict mode to tick 3912

*Decided 2026-11-15.*

**Unverified.** No capture covers any of it: the Goth captures end at
update 1000. Strict mode now runs through tick 3912 (08:41:08, hash
41652f2f…) and stops at tick 3913, where object 16 (tree 0x1010 node
6) asks for a route from inside a container (0x4832d4). The snack order
runs through step 4224 and stops on the same at step 4225. The snapshot
format does not change, and every summary line of `sims boot …
--ticks 3602` and every pinned hash up to tick 3602 is unchanged.

**Create New Object Instance's flag 0x4** (stop at tick 3603). The
flag was read as acting only at the end of the tail. It also picks the
type (0x44e0e6): with it, the definition is not the GUID operand's
(0x43ac10) but the one linked to the neighbor record the stack object
names (0x425afe: slot `id − 1` of the record vector, 0x4256f4, then
record +8, the definition its GUID resolves to). A stack object that
names no record leaves a null definition, and the false branch is
taken before anything is made (0x44e117). At the end of the tail
(0x44e7dd to 0x44e877) a new person that is a visitor (person word 32,
+0x5cc) runs the tree its file names "init visitor" (string 0x6c4120,
looked up by 0x4021f2) at once on its own thread: 0x474afb calls the
synchronous run 0x4749b5 with the person as its stack object and a null
arguments pointer, so its arguments are its own temps, as Run Tree By
Name's mode 1 runs. What it returns is not read.

**Mode 6 making a person.** Mode 6 clears the byte that asks for the
off-lot and room tests and the place (0x44e4b5, `[ebp-0x15]`), so a
person goes the tail's person steps and stays where the factory left
it: it takes its record's interests and personality (0x425930), flag
0x20 turns it, and the person-word test (84, 68, 74 → 0x45810c, not
ported) runs, as in mode 8.

At tick 3603 object 354 (GUID 0x20E32C3C, tree 0x1022) counts the
neighbor records, picks two at random, makes each one's person off
the lot (objects 334 and 336; node 29 and 31), writes their person
words 80 and 81, and removes both (nodes 33 and 34). The Goth lot is
residential, so neither is a visitor and no "init visitor" tree runs;
that step is checked only by the synthetic tests.

**Removing a person for another object.** A person's take-off
(0x433125) refreshes its animator (0x58da12) only if animator byte
+0x109 is set. The animator's tick writes that byte from the house
view's on-screen test (0x58f69f to 0x58f6e4: 1 after 0x5a0ef5 when the
test passes, 0 after 0x5a0ffd when it fails; the person's tick passes
1, 0x45cd39), and so does the view's draw (0x590b8b). It is not kept.
Until now Remove Object Instance took a person only when it removed
itself and yielded. Now any other removal of a person works the refresh
out on a copy of the lot as the take-off leaves it, as a person that
takes itself off does (`Interpreter::refresh_before_removal`). The
refresh must write nothing but the person's animator and base layer and
play no sound, and the person's hand must be empty. The lot goes on
without the refresh. Here it would place the never-placed animator at
the off-lot position and clear its skin's update mark.

The branches then differ only in that animator, which the deletion at
the end of the update destroys. What is checked before then:

- A person with an ID above the remover's has its tick in the same
  pass (the scheduler walks IDs in order). The refresh is held for that
  tick (`Interpreter::taken_off`, kept for the pass) and settled as for
  a person that takes itself off. Until its animator's tick, its thread
  may run only primitives that read no animator. The refresh then the
  tick must reach what the tick alone does.
- A person that removes itself in its own tick and goes on (flag bit
  0, or a synchronous run) has its animator's tick still to come at
  the same clock. The refresh is held in that tick
  (`Interpreter::refreshing`) and settled the same way.
- A queued person whose tick starts with no refresh held, or whose
  thread runs a primitive that reads an animator, fails.
- A head that looks at it fails, as before.
- A person left queued after a partial pass fails, as before.

Not checked, and **unverified** as for every removal of a person:
another object's primitive that reads the queued person's animator
before the deletion, and the deletion's last reset. In strict mode the two people are 334
and 336, below 354, so neither ticks again. In the snack order they are
374 and 375, so both tick while queued, and the branches agree.

**Tests.** `create_person` covers:

- the record lookup and its misses;
- mode 6 making a person;
- the "init visitor" run, and the cases where it does not run;
- the loud failures;
- the removal by another, with each guard;
- the pass in which the removed person ticks.

The old tests that pinned flag 0x4 and a mode-6 person as unported now
pin what still fails.
