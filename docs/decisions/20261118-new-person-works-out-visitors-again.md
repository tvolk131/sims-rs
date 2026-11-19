# A new person makes the lot work out again who is a visitor (signal 0xe2)

*Decided 2026-11-18.*

The Goth day (House05) matched track 2's recording `goth-day-20261115`
in its lot random stream through update 3602. At 3603 the original made
173 draws and the port fewer. In that update object 354 (tree 0x1022)
makes two neighbors' people off the lot (Create New Object Instance mode
6, flag 0x4), objects 12 and 24, and removes them both.

**The factory announces a new object.** At its end the object factory
0x440b2d sends signal 0xe2 with the new object's ID to the game's
subscribers (0x440eca to 0x440edc, through 0x40760d), whatever kind of
object it made; the person path (0x440d63) gets there too. The lot's
handler (0x41637f) takes 0xe2 at 0x4163f7: if the object table's entry
for that ID is a person (definition +0x12 is 2), it runs the visitor
pass 0x42a2b4. The interface's handler (0x4f670e) only sets a redraw
flag (+0x8f) unless its byte +0x8d is set. Whether any other subscriber
takes 0xe2 was not read in full and is **unverified**.

**The visitor pass (0x42a2b4)** is the one the household selection runs
at load (`World::finish_selection`, step 4). For each person in the
person vector, word 32 (+0x5cc) is read; a 0 becomes 1; it becomes 0 if
the person's type is one of the selected family's members (0x41019b,
family +0x12c); and on a lot that is not residential (0x40745f) a 0
becomes 1 again. The rule is now one function,
`sims_sim::load::visitor_again`, used by the load and by the factory.

The port's person factory (`Interpreter::create_person`) now runs the
pass after the person is made, as the signal does. The family is the
one global 9 names. That is the convention scope 24 field 1 already uses
(see `memory.rs`): every setter of global 9 to a family's ID also sets
the selected family's pointer. With global 9 at 0 the World does not say
which family the game holds, so the pass fails loudly
(`Trap::Unsupported`, naming 0x42a2b4), and a global 9 that names no
family fails as invalid. That includes the load: while the house is
read, 0x415fc6 clears global 9 and 0x415fd7 selects family 0, so a
person made then (for example by an object's Init before the household
selection) would be made in the original and fails loudly here. That
case is **unverified**: no capture shows one, and no ported load makes
one. With nobody in the person vector the pass returns before it reads
the family (0x42a2cc), so then nothing fails.

For a multi-tile object the signal names the group's lead, and the lot
tests the lead's definition. `Interpreter::create` runs the pass after a
group whose lead's definition has type 2. A single-tile object of type 2
is always made by the person factory. Any other object's 0xe2 changes
nothing in the simulation.

The construction reset's own test (0x4599f8, before 0x459a0b) goes
through 0x40745f too, for which lot 0 (global 10 at 0) has zoning 1
(0x42b9e3). The port took lot 0 there as not residential. Both now use
one test. The difference shows only on lot 0, for a person whose Init
leaves word 32 at a value other than 0 and 2: the reset used to make it
1, and now keeps it, as does the pass for someone not of the family.

**What it changes at 3603.** The two new people are not of the Goth
family, so the pass makes each a visitor (word 32 is 1). Create New
Object Instance's tail (0x44e7dd) then runs each one's "init visitor"
tree (0x200b), which makes seven Random Number draws. The port now makes
the original's 173 draws in update 3603.

**This corrects an earlier entry.**
`20261115-neighbors-people-made-off-lot-removed-strict.md` says that the
Goth lot is residential, so neither person is a visitor and no "init
visitor" tree runs. The lot is residential, but the factory's signal
makes both visitors, since neither is a member of the family. That entry's
port of the "init visitor" run itself was right.

**Checked against the recording.**

- All three random streams (lot, global and C runtime) match the
  recording after every update from 3603 to 4154.
- A comparison of each object's state with the recording's, every tenth
  update from 3600 to 4150, finds no difference in the fields it
  compares: data, temps, attributes, the last result, frames with their
  locals and arguments, and person data. The expected modelled unknowns of
  `20261118-set-to-next-follows-group-links.md` are left out. It does not
  compare motives, queues, relationships or other state, so a difference
  there would not show.
- From 3340 to 3590 the only difference in those fields is still person
  21's last result at 3350 (see
  `20261118-update-3350-is-pointer-pie-menu-listing.md`).
- At 4155 the global random stream moves in the recording, for the first
  time since the load, and not in the port. That is the next divergence,
  and it is not worked out here. At 4160 it is the only difference in the
compared fields. The recording's global stream moves after 4155, 4185,
4215, 4245, 4275, 4305, 4335 and 4365 (3, 21, 24, 24, 24, 24, 21 and 6
draws), and not otherwise; the port's never moves all day. A draw outside
the lot update draws from the global stream, so these are likely outside
it, which is **unverified**.

**Pins.** The strict boot (`--answers 3,3`) keeps its hashes at ticks
33, 1103, 2881, 3432 and 3602. From 3603 on every pin changes, in format
30 and in its format 29 twin. The whole-day test also checks the three
streams against the recording's values after 3603 and 4154.

| Tick | Format 30 | Format 29 |
|------|------|------|
| 3603 | 2c23d070…5b2c | c380dd8b…26a8 |
| 3912 | 610cc313…af63 | d77d3d23…a4c8 |
| 9906 | e9718f47…b8f0 | 63eb1dbb…8a89 |
| 20186 | 56f967f2…8bdd | 64ebcb01…d650 |
| 31478 | c1305044…6c76 | ab84d202…1ea1 |
| 43200 | 67169649…a7cf | 3594f77d…2334 |

The Newbies' house (House07, `--answers 3`) does the same at tick 1803
(object 290, tree 0x1022), so its pin at 4052 changes in formats 30,
29, 28 and 27 (cadfde0d…6af9, c55e174c…785e, 72fb2c74…3893,
cd9afec9…115a), and its 1802 pins stay. Its run still stops at the frame
after tick 4053, on a skin waiting for its update (0x590d26), but now
person 142's, not person 140's as
`20261115-seated-route-drop-pets-runs-route-failure.md` says. That is
unverified: the Newbies' recording ends at update 935.

At the day turn the objects' value is still 12524 (unverified, since
the run has left the recording by then). No World field is added and
the snapshot format does not change.

**Tests.** The interpreter fixture's world now has the family global 9
names (109), with the fixture Sim's type as its one member. New tests
cover the pass after a new person on a residential lot (a member gets
0, anyone else 1 for a 0, a 2 stays), with global 10 at 0, and on a lot
not zoned residential, and the loud failures; a group whose lead has
type 2; and the construction reset on lot 0. The "init visitor" test
now expects the tree to run for a non-member on a residential lot and not
for a member.

**What would reopen it.** A person made with global 9 at 0 (during the
house's load, or with no family selected) fails loudly here, and a
capture of one would show which family the original tests. The other
sender of 0xe2 (0x42184c, after 0x4a67de) is not ported.
