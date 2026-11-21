# A step runs both ways while a refresh is pending, and a dialog's portrait makes a person

*Decided 2026-11-20.*

Strict mode on the Goth house stopped at update 24716
(`20261119-generic-sims-call-6-and-the-saved-neighbor-record.md`). Three
changes take it through the whole day:

- a step that may read an animator while a refresh is pending runs both
  ways;
- a route from no room has no way;
- a dialog's portrait is built the way the interface builds it, on a
  tile searched for from the house view's centre, which is an input.

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

- once on a copy of the lot as it is;
- once on a copy that has the refreshed animator.

Each run also has its own copies of every cell the machine shares with
the rest of the tick (`both_ways::Cells`). Both runs must give the same
result and leave the same lot, apart from that person's animator. The
lots are compared by their encodings and by each thread's count of
restarts, which the encoding leaves out. The runs must also leave the
same in every cell:

- the questions asked;
- the objects deleted;
- the sounds;
- the pie menu;
- the action run at once;
- the objects let go;
- the objects being deleted;
- the people another object took off;
- the refresh pending.

Floats are compared bit for bit. An action's attenuation is a plain
`f32`, which `==` would call equal at -0.0 and 0.0 and `Debug` would call
equal for any two NaNs, so actions (the one run at once, and the pie
menu's) are compared written out with postcard, which keeps the bits.
Every other float in the cells is an `F32`, whose `==` compares bits.

If they do, the original reached the same state whichever branch it took.
The lot and the cells go on with the first run, and both animators are
kept, as each run left them, for the animator's tick to settle. If the
runs differ, or either run takes a person off the lot (itself, which
leaves a refresh pending in its copy, or another, which changes its copy
of the people taken off), the step fails loudly and names 0x58da12 and
byte +0x109. Neither run is kept then: the lot and the cells are as they
were before the step. When the runs agree on a fault, the first run is
kept with the fault, as any step that faults keeps what it did before
it. Keeping a run borrows every cell before it writes anything, so a cell
held elsewhere panics before the lot or any cell is kept.

A person queued for removal by another person's tick has its refresh
kept for the scheduler's pass (`taken_off`). A person's tick run on its
own (`Interpreter::tick_person`) gets a pass of its own for its whole
tick, its thread's run and its animator's tick alike. No other person's
tick follows in it, so a refresh left there at the end fails loudly
rather than being dropped, which would hide it from the comparison. A
tree run outside any person's tick still drops it.

The refresh itself (`Interpreter::refresh_at_take_off`) and the
animator's tick that settles the two animators
(`Interpreter::tick_animator_after_take_off`) work the same way. Each
branch runs on its own copies of the lot and the cells, and they must
agree in all of them. The tick alone is kept only when they do.

This stands in for the branch the original took, which is not known. It
is not a substitute for the refresh's timing, which is ported. Strict
parity is not weakened: a step whose outcome depends on the branch
stops the run.

`a_step_both_ways_that_differs_fails_loudly` makes the two runs disagree
and checks that the step fails. In it, a clip's timed event is due at
the clock the refresh reads, so only the refreshed animator has the event
queued. Animate Sim then takes the event on one run and waits on the
other. `a_step_both_ways_that_fails_keeps_neither_run` has a step that
takes the person off the lot again, and checks that the lot is left as
the take-off left it.
`after_taking_itself_off_a_step_that_agrees_both_ways_goes_on` is the
agreeing case. `actions_are_compared_bit_for_bit`,
`a_cell_held_elsewhere_stops_the_keep_before_anything_is_kept` and
`a_person_removed_in_a_persons_tick_outside_the_schedulers_pass_fails_loudly`
cover the comparison, the keep and the removal above. A run can take another person off only through Remove
Object Instance, which never runs both ways, or a tree that a primitive
run both ways calls. No synthetic test reaches that case. The check is
there so that such a run fails rather than passes.

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

- **Text tokens.** This part is ported. The record's strings come from
  STR# 301 of the running code's file (0x4360b8, 0x47028b):
  - the buttons' labels its kind loads (0x436175);
  - the title (operand 6);
  - the message (operand 2).

  Each string is formatted in that order (0x44bf1a, from 0x436219 to
  0x436265), with one place for a type, cleared first (0x43620d). Only
  two tokens write that place:
  - `$Object` (0x44c1af) gives the stack object's type;
  - `$Neighbor` (0x44c69f) gives the type of the neighbor record that
    the stack object names (0x425afe). Its pass comes after `$Object`'s.
    It finds `$Neighbor` anywhere in the text as the passes before it
    left it. The `$NeighborLocal` pass just before it (0x44c590 to
    0x44c65a) replaces a token with a person type's name when the
    token's number is one of the tree's locals, the local names a
    neighbor record (0x4256f4), and the record's type is in the
    interface's table of person types (0x4e9753). A record whose type is
    not in the table ends that pass (0x44c616). A token not replaced
    stays, and `$Neighbor` is found in it.
    - The token's number is read with the C runtime's 32-bit conversion
      (0x44caf6, 0x636029), which wraps, and compared with the locals'
      count signed.
    - The local is read as any read of a local is (0x44c5f4), so one
      that starts as half of a pointer the original left fails.
    - The type's name comes from the interface's table (0x4e9c46,
      0x4e893e), which is not ported. Where a name took a token's place
      and the `$Neighbor` search after it would then decide between two
      types, the port fails loudly: the name could hold `$Neighbor`.

    Tests: `a_neighbor_local_named_in_a_dialogs_text_is_not_its_neighbor`,
    `a_neighbor_local_the_original_left_unknown_fails_loudly`.

  The last type found is kept, and mode 0 shows it in place of the stack
  object's (0x436308). Mode 2 does not read it. Two kinds of string are
  taken to hold no token, which is **unverified**: a string the file
  lacks, and a button's default label (0x435f4e). The first version of
  this change failed on any `$` in a mode 0 dialog's strings. On the Goth
  house that stopped the run at update 901 (object 8, tree 0x101d), so
  the tokens that give the type are ported instead.
- **Placement.** The put (virtual function 0x88, 0x45e4e9) is the
  placement 0x433340, on a tile the interface searches for (0x4347f0,
  with the defaults of 0x4347d1: a spiral on the ground level, in the
  start tile's room, on an empty tile, no random start) from the house
  view's centre: the view's pick at the centre of its rectangle
  (0x4a677c, 0x4a7b58). The camera is the player's, so the tile at the
  view's centre is an input, as the view's rotation is
  (`Interpreter::view_centre`, `Lot::set_view_centre`, `sims boot
  --view-centre X,Y`, saved with a lot from save format 2). The pick
  itself is not ported, so the input is the tile it gives. When the tile
  is given, the search runs from it as in the original, its place tests'
  callbacks included. When it is not, the search runs from every tile it
  may start from (`Interpreter::search_from_anywhere`), on a copy of the
  lot and of the machine's cells. Every callback tree the place tests
  run (function 5 of the person and of the objects in its way, 0x434b94,
  0x433a1a, 0x433a35) must then leave both as they were, since which
  tiles the original tries depends on the start. In practice any callback
  run fails this: it leaves its frame's words in the object's stack
  storage (`FrameStorage`), which is part of the lot, unless that storage
  held the same words already. So without a view centre the portrait
  stops at the first callback its search runs. At the Goths' 24719 none
  is run.

  `Interpreter::portrait_put` does what is the same on every tile the
  search may end on: the interface's work for an object come from
  outside (0x44752e, `placed_from_outside`). The person's removal is
  taken to undo the rest of the put (the rooms, 0x440385; the tiles,
  0x4330d7; **unverified**). It fails loudly where the tile would leave
  the lot otherwise than the removal finds it:
  - the person's function 9 (0x433580), 6 (walls changed, 0x435110),
    25 or 26 names a tree. A person with function 25 or 26 is a seat:
    the objects on the tiles beside it are told, and may be asked their
    function 9 (0x43355d, 0x433612). So does its function 10 (before the
    removal, 0x433148, which sees the room it was put in) or 5 (0x4337f6,
    which the search asks). Its Init (function 0) runs when it is made,
    as for any person;
  - its data word 7 is not 0, which marks the room it is put in and taken
    from as not counted for its light (0x433ed8, 0x4412c6);
  - its data word 40 has bit 0x400, which marks the room it lands in as
    not counted (0x433340);
  - it is a person of kind 0 or 1 (person word 65; a pet is above 1) and
    the tile the search ends on (with no view centre, any it may end on)
    is in an indoor room that is not a pool and not marked as occupied.
    The put marks the room (0x45e43d, 0x468802), and its removal leaves
    the mark;
  - no tile is found (from the view centre given, or with none from some
    start). The original then removes it without drawing it (0x4e9706),
    and 0x4e938c builds again in the same dialog unless the type has
    image 2002. That is not ported.

  The build runs on a copy of the lot, which is kept only when it is
  done, so a build that fails leaves no person behind (the original
  removes it on every path, 0x4e8fdb, 0x4e9706). Tests:
  `a_portrait_whose_person_runs_code_when_put_down_fails_loudly`,
  `a_portrait_whose_put_depends_on_its_tile_fails_loudly`,
  `a_portrait_searches_from_the_view_centre_when_it_is_given`,
  `a_callback_the_search_runs_must_change_nothing_when_no_centre_is_given`.
- **`$Object`'s name.** `$Object` is replaced by the stack object's
  catalog name (0x44c1ca, 0x4573d9 to 0x457d3b): an entry of the CTSS
  table definition word 41 names, read through 0x4061e3, or the
  definition's own name when that entry is empty (0x457db4). The
  `$Neighbor` pass after it reads the name too, so a name that holds
  `$Neighbor` shows the neighbor's type. Which entry 0x4061e3 picks is
  not ported, so a name is taken to be plain only when its table has an
  entry in language 1 and no entry is empty or holds `$Neighbor`. Where
  the name would decide the type, the dialog fails loudly. On the Goth
  house this matters at update 901. Test:
  `an_object_name_that_may_hold_a_neighbor_token_fails_loudly`.
- **Other builders.** Only the dialog's builds are counted. The household
  panel draws the family's portraits, so a portrait of a family member's
  type with nobody of it on the lot fails loudly. A type made while the
  game ran (`World::characters`) fails loudly too, since the table may
  not have it. The relationship panel and other interface builders are
  not counted. In the recording nothing else built the cat's image.

## The view's centre at 24719, and the stand-in

At 24719 the Goths' cat portrait is built with nobody of its type on the
lot, so the search runs. Of the 3,844 tiles it may start from (inside the
border of a 64-tile lot), four find no tile: 19,24; 23,31; 26,28 and
26,31, each in room -5 (none). From every other start a tile is found,
and the build leaves the same lot. The fitted-frame test checks this
(`every_start_gives_one_state`). At 24718 it saves the lot, steps 24719
once from each start, and checks two things: the four starts stop where
the original would build again, and all the others reach one state hash.
So the start does not matter outside those four cells.

The recording's state shows a single 1,920-draw build at 24719 and no
second one. So the real camera was not on one of the four cells, and
every other start gives the state the recording implies. Without the
input, a boot of the Goths stops at 24719, as the adoption dialog at
17707 does without `--answers`. The Goth day's tests give the lot's
middle tile, 32,32 (`GOTH_VIEW_CENTRE`). It is labelled in the code, the
tests and here as **a stand-in start tile, not the original's camera**.

Open items:

- read the house view's centre from a capture (scroll, zoom, rotation);
- port the screen-centre pick (0x4a7b58);
- both tie to the open question of which camera the original opens the
  Goths' house with.

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
