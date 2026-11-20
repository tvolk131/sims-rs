# Generic Sims Call 6 and the saved neighbor record

*Decided 2026-11-19.*

Strict mode on the Goth house stopped at update 17929. There person 18's
tree 0x0189 runs Generic Sims Call routine 6. Porting that routine let
the run go on, and the next divergence, at update 21237, came from a
record save that Remove Object Instance makes. With both ported, the
lot and C runtime streams match the recording `goth-day-20261115` through
update 24715.

## Routine 6 (0x44fc53)

The routine takes the neighbor that the stack object names out of its
family. The stack object names it by the record's place plus one
(0x4256f4). If no record is at that place, the routine throws native
error 0x37.

It first sends the Escape key to the interface's top window when the
window manager still holds that window (0x4b4491, 0x5da49f). This is taken
to be presentation. No modal dialog is open while scripts run, so it
could close nothing the simulation keeps. **Unverified.**

It then calls 0x42601e, which fails and takes the false branch in three
cases:

- the record's family word (61) is 0;
- no family has that ID;
- the record's GUID is not one of the family's members.

Otherwise 0x42601e does the following, in this order:

1. The first live person of that GUID gets word 61 set to 0.
2. The record's word 61 is set to 0.
3. The first member with that GUID is taken out of the family.
4. Who is a visitor is worked out again (0x42a2b4, the same pass the
   factory's signal 0xe2 runs; see
   `20261118-new-person-works-out-visitors-again.md`).
5. The family's flag 8 is cleared. The World does not hold the family's
   flags, so a script that reads them fails loudly.
6. The family's friend count is marked out of date.
7. Signal 0xe8 is sent. Only the interface's handler has a case for it,
   and that case redraws the control panel.

The routine then takes the true branch.

## Remove Object Instance saves the removed person (0x451bee to 0x451c03)

After 0x442d70 queues the object, Remove Object Instance casts the object
it named to a person. If the cast succeeds, it calls 0x425a69 on it. This
is the named object, not the lead tile.

0x425a69 saves the person into the neighborhood record that person word
31 names (0x4256f4). It does not save into the first record of the
person's type. If word 31 names no record, nothing is saved. The save:

- sets the record's revision to 9 (0x423e04);
- copies every entry of the table 0x423827 builds for revision 9.

The copy has no per-entry revision test. The attach at 0x4258ff has one.
Other callers of 0x425a69 exist:

- routine 25 (0x450c02);
- 0x45a63b;
- the house save (0x42e116, and the person's virtual function +0x48
  through 0x43fb80);
- the interface (0x4ea538).

These are not ported here.

The save matters when another person of the same type is made later. The
post-constructor (0x45962b) and the construction reset (0x4598e8) draw
default traits (0x459a9c) only while interests 46 to 53 are 10 or less.
At update 21237 the Goths' lot makes a person of type 0xA23F4AEA. A
person of that type, person 246, was made at 16558 and removed at 18072.
In the original, that removal saved person 246 into record 4. The new
person attaches that record, finds its interests set, and draws no
traits. Without the save, the port drew the traits twice (91 extra C
runtime draws), and the streams diverged from there.

The save reads person words. The animator's refresh at the take-off,
which may still be pending when the save runs, writes only words 41 to 45
and 64. None of those is in the revision 9 table. The save can therefore
run with the refresh pending.

## Primitives that may run with an animator refresh pending

A person that takes itself off the lot has its animator's refresh pending
until its animator's tick. Until then, threads may run only primitives
that read no animator (`placement::reads_no_animator`). Five more are
allowed now. The reason for each is a code comment:

- **Random Number (8)** reads and writes through the same memory reader as
  Expression and draws from the lot's stream.
- **Idle For Input (17)** reads the person's words, motives and queue, and
  the lot's zoning. The check and cancellation trees it runs, and the
  tree it pushes, are checked one primitive at a time. Making an action
  the running one (0x45e6cd) works the person's footprint out again when
  person word 73 is set (0x45e756, 0x433d16). That goes through 0x43519f,
  which calls only 0x4351ef, and that reads person words. The virtual
  call at 0x4353c9 is in a separate routine (0x4352e8).
- **Run Tree By Name (28)**, handled at 0x44ba26, looks a tree up and
  either runs it at once or pushes it. It also counts the use (0x45e59b).
  The tree it runs is still checked one primitive at a time.
- **Play Sound Event (23)** (0x452822) hands a name and an object ID to
  the sound manager.
- **Stop All Sounds (48)** (0x45253a) hands it an ID.

For Play Sound Event and Stop All Sounds, whether the sound manager reads
anything the refresh writes is **unverified**. It would change only what
is heard.

**Go To Routing Slot (45)** is refused only where it would first read the
animator, not at the step. 0x40743b and 0x432595 read only the stack
object's own position, container and +0x110. As a result, a stack object
that is off the lot gives false whether or not a refresh is pending.
Naming the slot reads no animator either: a source that names no slot
raises native error 0x25 (0x45ace4) first. The route that follows reads
the person's animator.

**Remove Object Instance (18)** checks for itself. Only the person whose
refresh is pending may remove, and only itself, yielding. A removal made
on another thread meanwhile is refused, for example by a cancellation
callback that Idle For Input runs on the action's target.

## Effect on the pins

The record save changes the World's neighborhood records whenever a
person is removed. Every strict pin from Goth update 2881 on moved for
that reason, and so did the Newbies' pins at 1802 and 4052. With the save
left out, all of the earlier pins held, and the only change was the
divergence at 21237. The pins were taken again with the save in.

The fitted-frame Goth test now checks the streams at 16112, 17562, 17928,
21237 and 24715, in addition to its earlier checkpoints.

## The three other houses, updates 1 to 1000

The Newbies (`house07-ticks-20261115`), the Pleasants
(`house06-pleasants-ticks-20261115`) and the Roomies
(`house10-roomies-ticks-20261115`) were compared with this change at
every update from 1 to 1000. The compare covers:

- the three streams;
- the scheduler's cursor;
- the globals;
- each object's data, temps, attributes, last result, person data and
  frames.

Neighborhood records are not compared. The state dumped from this change
was byte-identical to `main`'s (1241fcf2) at updates 1, 500 and 1000 in
all three houses. All three run all 1000 updates in strict mode.

- **The Roomies** match at every update.
- **The Newbies** match, except for global 12 at updates 1 and 3
  (native 17, here 0). This is the known difference after the first
  tick (`20261114-comparison-harness-runs-again-capture-holds-1000.md`).
- **The Pleasants** match, except for three differences:
  - the two differences their load already had
    (`20261115-household-selection-works-out-again-who-visitor.md`):
    object 243's word 50 (updates 1 to 116) and its last result (in 143
    updates from 1 to 614), and bit 15 of person 241's fourth frame's
    tree word (updates 1 to 945);
  - person 241's tenth frame (tree 0x2018, nodes 5 and 7). Its local 1
    is 3270 in the original and 0 here, from update 669 to 928. This
    difference is on `main` too and is not looked into here.

## The sound test

`sims-game`'s Goth sound test runs the lot until it stops. Before this
change it stopped at routine 6. Now it runs (with default frames) to
update 167,658. Over that run, the fountain (object 197) plays
"fountain_tinkle" every 300 updates. Each play starts a thread of track
1346 that does not end, so at about update 74,700 the player reaches its
own limit of 256 threads. How the original treats a play of a track that
its object is still playing is **unverified**. The test now runs at most
a whole day (43,200 updates).

## Next stop

At update 24716 the run stops on person 12 at tree 0x2040 node 7. Earlier
in the same tick person 12 took itself off the lot. It then runs Go To
Routing Slot before its animator's tick, and passes the test of whether
its stack object is on the lot. What the refresh at the take-off
(0x58da12) writes, if the house view drew the person, is not ported. That
is the next piece of work.
