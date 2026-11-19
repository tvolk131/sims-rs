# Set To Next mode 3 follows a group's links, and four ports that take strict mode through the day again

*Decided 2026-11-18.*

The Goth day (House05) matched track 2's recording `goth-day-20261115`
through tick 2810 for every object's state, and through tick 3602 for the
lot's random draws. At update 2811 the original and the port walk the
school bus's tiles in different orders. With that fixed (section 1),
every tenth update from 2820 to 3340 matches the recording, apart from
the differences listed below. The run after the
fix is a different one, and four more ports (sections 2 to 5) let strict
mode run the whole day again.

**1. Set To Next mode 3 follows the group's links (0x44ec79).** Mode 3
finds the next tile of the cursor's multi-tile group. The port walked
`World::order` and took the next object of the same group. The original
casts the cursor to a group tile (0x6c0ef8), reads its next link (+0x18c,
`World::next_parts`) and, at the last tile, the lead (+0x190,
`Multipart::parent`). The two orders agree when the tiles were
registered in link order. The school bus is made at run time (tick 2703, by the carpool), and the factory
registers its tiles in the opposite order from its links. Tree 0x1019
loops over the bus with mode 3. The port visited tiles 12, 29, 28, 27,
26, 24, while the original visits 12, 24, 26, 27, 28, 29. A cursor whose
definition word 10 (+0x118 +0x14) is 0, or that names no object, gives 0
and the false branch, as before. A cursor whose word 10 is set but that
is no tile would be read through a null cast in the original (0x44ecaa).
That fails loudly.

After the fix, a comparison of the port's full state with the
recording's at every tenth update from 2820 to 3360 finds only these
differences (the third at 3350, which is why the match is claimed only
through 3340):

- **Expected.** Frame local 2 of tree 0x1019 at 2820: the native value
  is −10096, and the port's is one of the modelled unknowns
  (`frame_storage[id].unknown`). It overlays a frame header word that
  holds a code-owner pointer.
- **Expected.** Person 21's word 37 from 2950 on: the native value is
  3120, and the port's is one of the modelled unknowns
  (`routing.unknown_words`). 0x45c418 writes it from uninitialized
  memory.
- **Open, unverified.** Person 21's last result at 3350: the native value
  is 1, the port's 0. They match again at 3360. Person 21 is in Go To
  Routing Slot step 3, at tree 0x1019 node 5. In the port its route ends
  true at tick count 91800 (update 3356) and the person is taken off
  the lot at (−16, −16). Why the original's result is already 1 at 3350
  is not worked out.

From 3610 on the lot's random stream differs. That is the divergence
already known at tick 3603, which this fix does not touch.

**2. A model matrix off the lot stands at height 0 (0x45ee1d).** At tick
15750 a person off the lot needs its model matrix. 0x45ee1d tests each
rounded coordinate's tile, as a signed byte, against 0 to the lot's size.
Off the lot, the original skips the altitude lookup, and the height stays
0 (0x592245). The level's rise is still added. This used to fail loudly.
The same point in play mode is the snack-order test's step 15799, which
now runs without an error.

**3. The adoption call is answered "No".** At tick 17707 the neighbor
controller (object 5, GUID 0xe25246d2, tree 0x100b "phone call dialog
with response") runs on neighbor 24 the call that asks "Would you like
to adopt a baby?" (STR 301): a dialog of kind 1 whose answers are 2
(Yes) and 3 (No). Strict mode's answers are a queue given at load, so
the whole-day test now loads with `&[3, 3]`, and the second 3 answers
this call. Saying no keeps the household as it is. That is a harness
choice. Whether the recording's player would see this call at all is
**unverified**: the run has left the recording by then.

**4. A route whose stack object is lost fails (0x45ae8d).** At tick
18170 object 26's route (tree 0x1012 node 1) finds its stack object gone.
On every run after the first, Go To Routing Slot and Go To Relative
Position test the stack object (0x40743b, then 0x432595 for standing on
the lot). If it is gone, the step becomes 19, or 20 if the route was
under way (step 3). Then the run goes on to the byte +0x78 test
(0x45ae9c) and the jump table at 0x45b9d4:

- Step 20 (0x45b352) asks the animator to stop (0x589e2d). If it does
  not, the run keeps the step and yields (0x45b9a0). Once it stops, the
  next step is 19. No sound plays, unlike steps 10 and 11. The next run
  tests the stack object again first, so if it is still lost, the saved
  step 20 becomes 19, and the route fails without waiting any longer
  for the animator.
- Step 19 (0x45afc4) goes to step 7: the route fails.

With byte +0x78 of the context clear, any step but 2 and 3 only sets it
again (0x45aebb) and goes on. Steps 2 and 3 still fail loudly
(0x45aea5). The end of the route reads nothing of the lost object.

The lost test is 0x432595 (`placement::stands_on_lot`). It used the
port's `on_lot`, which takes the lot as 64 tiles across, but 0x4325b5
calls 0x422632, which tests against the lot's own size (lot +0x10). So
`stands_on_lot` now uses `off_lot`, which has the native bound. Its
other callers (the daily update, the rooms' lights, Burn, dynamic
groups) call 0x432595 too, so they take the same bound. No Goth pin
changes with it. The two bounds differ only for an object on a lot
smaller than 64 tiles, standing between the lot's edge and the arrays'
edge. `on_lot` itself, which other placements use directly, is left as
it was (unverified there) for its own change.

**5. A route's end cancels the asked person's running action
(0x45d617).** At tick 24605 object 28's route ends while the person it
asked to move (context +0x80) is running the action it asked for
(+0x90). The end of a route calls that person's 0x45d5f8. For the running
action, that is the player's cancel of a running action:

- person word 33 becomes 0;
- the action's flag 0x100 is set;
- unless the person would be interrupted anyway (0x45d7c0), an action
  with menu number 33 is made (0x41f00e). It is queued (0x45d20b) only
  if the person's own interaction table has an entry with that menu
  number (0x41f0ff).

`Machine::release_action` now runs `cancel::cancel_running` on a machine
for the asked person. The method is the person's own, so it reads that
person's data.

**6. Create New Object Instance mode 3 (0x44e34c).** At tick 37803
object 359 (tree 0x100a) makes an object with mode 3 and flags 0x19.
Mode 3 is mode 0's tile ahead, but of the stack object: its position
moves one tile in its facing (0x42228a) on its level. Unlike mode 0, the
room filter is the stack object's room (data word 29, +0x84). So the
tail's room test runs, as it does for mode 4 (0x44e582). A stack object
that names no object takes the false branch before anything is made.
Flags 0x2 and 0x40 still fail loudly.

**Pins.** With these, strict mode runs all 43,200 updates again. Ticks
33 and 1103 keep their hashes. From tick 2881 on, every pin changes,
since the run differs from update 2811:

| Tick | Hash |
|------|------|
| 2881 | 676bb460…085a |
| 3432 | 3a1e59fa…6d8f |
| 3602 | bbcf10f5…ba67 |
| 3603 | ec3305de…5981 |
| 3912 | 5b7d8a7d…ceeb |
| 9906 | ebb7406a…b30e |
| 20186 | 1627224c…55f7 |
| 31478 | 34ab8467…bc7b |
| 43200 | 9126004c…e644 |

At the day turn the objects' value comes to 12524, not 12653. That value
is unverified, since the run has left the recording.

No snapshot format changes: no World field is added.

**What would reopen it.** A recording past tick 3603, once the
random-draw divergence there is fixed, would check sections 2 to 6 and
the adoption answer. The open last-result difference at update 3350 may
point to a route detail that is not yet ported. This entry adds to
`20261115-snap-offset-order.md`, which still describes 0x45ee1d as not
ported.
