# A step runs both ways while a refresh is pending, and a dialog's portrait makes a person

*Decided 2026-11-20.*

Strict mode on the Goth house stopped at update 24716
(`20261119-generic-sims-call-6-and-the-saved-neighbor-record.md`). Three
changes take it through the whole day:

- a step that may read an animator while a refresh is pending runs both
  ways;
- a route from no room has no way;
- a dialog's portrait is built the way the interface builds it.

With these, the C runtime stream matches the recording
`goth-day-20261115` at every update of the day. The lot stream matches
through update 36717. After that it moves one update early or late at a
few updates (see below).

## The refresh's branch is not known, so a step runs both ways

A person that takes itself off the lot in its own tick has its animator
refreshed at the take-off (0x433125, 0x58da12). The refresh is made only
if animator byte +0x109 is set, which happens when the house view drew
the person at its last draw. That byte is presentation and is not kept.

The disassembly gives the refresh's timing: it runs at the take-off,
and the port already works it out there (`Interpreter::refresh_at_take_off`).
What the port cannot know is whether the refresh was made at all. Until
the person's animator's tick, the port keeps both animators: the one the
take-off left, and the one the refresh would have left
(`placement::Pending`).

So far, only primitives that read no animator were allowed to run in
between (`placement::reads_no_animator`), and any other primitive failed.
Now any other primitive, except Remove Object Instance (which checks for
itself), runs twice (`Machine::step_both_ways`, `crates/sims-vm/src/both_ways.rs`):

- once on the lot as it is;
- once on a copy that has the refreshed animator.

Both runs must give the same result and leave the same lot, apart from
that person's animator. They must also leave the same in every cell the
machine shares with the rest of the tick:

- the questions asked;
- the objects deleted;
- the sounds;
- the pie menu;
- the action run at once;
- the objects let go;
- the objects being deleted;
- the people another object took off.

If they do, the original reached the same state whichever branch it took.
The lot goes on with the first run, and both animators are kept, as each
run left them, for the animator's tick to settle. If the runs differ, or
either run takes a person off the lot (which would leave another refresh
pending), the step fails loudly and names 0x58da12 and byte +0x109.

This stands in for the branch the original took, which is not known. It
is not a substitute for the refresh's timing, which is ported. Strict
parity is not weakened: a step whose outcome depends on the branch
stops the run.

`a_step_both_ways_that_differs_fails_loudly` makes the two runs disagree
and checks that the step fails. In it, a clip's timed event is due at
the clock the refresh reads, so only the refreshed animator has the event
queued. Animate Sim then takes the event on one run and waits on the
other. `after_taking_itself_off_a_step_that_agrees_both_ways_goes_on`
is the agreeing case.

The refresh lasts only for the person's own tick (`Refreshing`), and the
people another object took off last only for the scheduler pass
(`taken_off`). Carrying two branches further would need a World-side
store with a format bump, and it would need the animator's tick run both
ways until the branches meet. #255's two person-reset branches would need
both, and neither is done here.

## A route from no room has no way (0x45c3ca to 0x45c3e4)

Route to slot (0x45c38e) looks up the person's room (data word 29) in
the lot's rooms (0x466fcc, a map's find that gives null at its end). A
person in no room, such as one off the lot, gets the answer 0 (0x45c4c7),
and the candidates are dropped (0x481898), as when the planner finds no
way. The port used to fail there.

At 24716, person 12 has taken itself off the lot and runs Go To Routing
Slot (tree 0x2040 node 7). The step runs both ways, and both runs answer
0 before reading the animator.

## A dialog's portrait (0x43601c, 0x4e92a8, 0x4e8c5d)

At 24719, object 18 (GUID 0x512C81F6, made at 24716) shows a dialog
(tree 0x100a node 1, operands [0,0,1,0,0,0,2,4]) with icon mode 2: the
neighbor record that the stack object (328) names. The recording's C
runtime stream moves 1,920 draws there, one cat skeleton's copy
(0x598da3). No object is made or removed in the recorded states at 24718
and 24719.

The ordinary dialog record's constructor (0x43601c, from 0x4443ab's
default case) picks the person whose portrait it shows (0x436279):

- mode 0: the stack object's type;
- mode 2: the neighbor record's type (0x425afe, record +8).

It then has the interface build that type's portrait:

1. 0x4e9753 finds the type in the interface's table of person types
   (0x6f2698). At startup the table gets one entry for each person
   definition in the catalog (0x4e8b3d), and 0x4e8a44 adds entries for
   later ones.
2. 0x4e92a8 builds the entry's first image if it has none.
3. 0x4e8c5d draws the first person of that type on the lot (0x4e9608).
4. If there is no such person, it makes one (0x4e9652, the factory
   0x440b2d), puts it on a tile near the house view's centre (0x4347f0
   from 0x4a677c, then virtual function 0x88), draws it, and removes it
   at once (0x4e8fdb, 0x44123f).

`World::portraits` (snapshot format 31) keeps which types have their
image. The Dialog primitive does what 0x4e92a8 does when it makes a
record (`Machine::dialog_portrait`). In the recording the cat's type had
no image yet, so a cat was made and removed. That accounts for the
1,920 draws, and the objects after it match the recording.

What is not ported:

- **Text tokens.** In mode 0, the dialog's text can name a neighbor whose
  portrait is shown instead (0x44bf1a). Text is not modeled, so the port
  builds the stack object's portrait. **Unverified.**
- **Placement.** The person made is not put on a tile, because the house
  view's centre is presentation. Nothing runs between the put and the
  removal. If the original finds no tile, it removes the person without
  drawing it, and the entry stays without its image. The port counts the
  image as built. **Unverified.**
- **Other builders.** Only the dialog's builds are counted. The household
  panel draws the family's portraits, so a portrait of a family member's
  type with nobody of it on the lot fails loudly. A type made while the
  game ran (`World::characters`) fails loudly too, since the table may
  not have it. The relationship panel and other interface builders are
  not counted. In the recording nothing else built the cat's image.

## The cat at 24719 was the portrait

`READ_BEFORE_THE_FIRST_LOAD` lists the kat file, because each cat made
takes one skeleton's worth of draws. Its doc used to count 24719 among
the cats made, from the draws alone. That was the portrait, and the doc
now says so. The dog file is listed too (#275), and the dog the Goths'
lot makes at 24766 draws 1,776, as in the recording.

## The rest of the day

Under the fitted frames, the C runtime stream matches the recording at
every update to 43200. The lot stream matches through 36717. At a few
updates after that (36718, 36929, 37058, 37126, 37215, 41445, 41858 to
41892, 42724, 42745, 42948 and 43003) it moves one update before or after
the recording's, and it matches again at the next update. Each time it is
the same draws a frame early or late, which is taken to be the recording's
frames drifting against the fitted schedule. **Unverified.**

Objects match the recording at 24716, 24718 to 24720, 24765 to 24767 and
24800. The only difference is the global stream, which is the censor
mosaic (`20261118-goth-day-global-draws-are-censor-mosaic.md`). The
fitted-frame test now checks the streams at 24716, 24719, 24766, 36717
and 43200.

## Effect on the pins

`World::portraits` is a new field at the end, so the snapshot format goes
to 31 and every pin was taken again. Each pinned test also checks that the
format-30 encoding equals `main`'s pin. That holds at every pin of the
default-frames Goth day and the Newbies, so those runs are unchanged apart
from the new field. In particular, no person is made for a portrait on
those runs: each type they build has someone on the lot.
