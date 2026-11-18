# Snap's slot offset in the original's order, the effect frame break, and the model matrix upstairs

*Decided 2026-11-15.*

The Goth day (House05, answers 3) left track 2's recording
`goth-day-20261115` at tick 2515. Three fixes take the day to tick 2810
against the recording, and a fourth lets strict mode past tick 8728.
Six more ports (sections 5 to 10) take strict mode through the whole day
again, all 43,200 updates.

**1. Snap mode 3's slot offset (0x454b93).** A slot that puts me into
no container slot places me at the object's position plus the slot's
offset turned to the object's facing. The original hands 0x454f5c the
object's +0xf4 first, then +0xf8. Those are our `position[1]` and
`position[0]`. The place comes back in the same order: the first
coordinate goes back to +0xf4 (0x454c3f). The port had both swapped.
That put a person at the wrong side of a slot whenever the offset's two
coordinates differed after the turn. At tick 2515 person 16 snapped to a
slot of a chair and landed 16 sixteenths away from where the original
put them.

**2. Special Effect is shown.** The options' "show effects" flag (+4 of
the options at 0x6f2628, read by 0x4cfe18) is 1 in the frozen image of the
running game (0x6f262c). The options' constructor (0x4e5f63) writes 0,
but the saved options turn the flag on. An earlier entry said the image
held 0, and that was wrong. The recording agrees. At tick 2747 objects 12
and 334 run Special Effect (flags 0x43) on the Goth lot, and state 1
yields once more before their next sleep. Only a shown effect does that.
The Options panel's check box is not ported, so the flag is a constant
(`OPTION_SHOWS_EFFECTS`, on).

**3. Global 11 ends the idle call.** Special Effect's first state sets
global 11 (0x44aad6), and nothing else reads it. The idle call (0x46d9d6)
runs updates (0x46da70, 0x46d345) until its time is spent. After each
one it checks global 11: when it is set, the call clears it (0x46dacc)
and returns (0x46da98), leaving the rest of its time to the next idle
call. That call sets the idle speed again, which changes nothing more,
and goes on with the updates. So `Lot::advance` clears global 11 after
each update, and no step ends with it set.

The call also returns, without clearing it, when 0x6e54a8 is set (0x46da90),
or when the simulator's byte +0xd4 is set and a message is waiting
(0x46da75, 0x46da86). Neither is modelled, so the port takes it that
neither happens (**unverified**). Between the two calls the
message loop runs, and it draws only when the draw timer says so. Our
run's draws are its input schedule (`Frames`), so no draw is added for
the break. Whether a draw falls there in the original, and whether it
moves anything a later update reads, is **unverified**. The recording
through tick 2810 covers the break at tick 2747. An earlier version of
this port cleared the flag before the next update instead, so a step
that set it ended with it set. The hashes pinned here are the same
either way, since no pin falls at the end of such a step.

**4. The model matrix above the ground (0x5922ad).** With the first three
fixes, person 18 goes up to level 2 at tick 8728, and the head's model
matrix failed loudly there. The original adds to the position the
level's rise, a vector from the house view's table:

- The view (`[[0x6e48bc]+4]`) is made by 0x4a6b15 with vtable 0x668744.
  Its virtual function +0x14 reads the zoom (0x6f00d8), +0x18 reads the
  rotation (0x6f00dc), and +0x24 returns the view's +0xb8.
- The table holds one 3-float vector per zoom, at +0xb8 less 12 bytes per
  zoom step. The view's setup fills every one with the same vector: a
  half, 9.300511 (0x4114cee5) and a half (0x4ca054). So the zoom does
  not matter.
- The rise is the vector times the levels above the ground. It is then
  turned about the second axis by minus the rotation times pi times a
  half (0x5c3a20, 0x5c39a0), and added to the position in world units
  (0x59237b to 0x59238e).
- 0x6f00dc is the same rotation as the scene's +0x84, which
  `Interpreter::view_rotation` models: every write of 0x6f00dc hands the
  same value to the scene's 0x4791a6 or copies it from there (0x4a6d58,
  0x4a7f28, 0x4a87d7).
- At rotation 0 the turn is the identity, so the rise is used as it is
  (`sims_sim::head::placement_on_level`). Any other rotation above the
  ground fails loudly at 0x5c3a20, until the turn's float steps are
  ported.
- The terrain under the person is added on every level, as before.
- 0x478f76, called on the way when the person's flags allow, only reads
  the tile, and its result is not used.

**Against the recording.** With the fixes, our run and the recording
match as follows:

- The lot's random stream matches through tick 3602. Before the fixes it
  differed from tick 2515.
- Every object's full state (data, temps, attributes, frames, person
  data and relationships) matches through tick 2810. The exceptions are
  the known last bit of persons 16 and 18's head target and neighborhood
  record 301.
- From tick 2811 person 16's frame 8 (tree 0x1019 on object 28, a loop
  over tiles with Set To Next mode 0 and Go To Routing Slot) holds stack
  object 24 where the original holds 27, and person word 37 is 32 where
  the original holds 4. That is the next divergence, and it is not
  looked into here.

The upstairs model matrix is not checked against a capture: our run has
left the recording well before tick 8728.

**After the fixes, strict mode stopped at tick 20187.** Before the
fixes the run reached a whole day, but it went another way after tick
2515, so it never needed what follows. Each port below lets strict mode
go on to the next stop. They are checked against the synthetic tests
only, since the run has left the recording by then (**unverified**).

**5. A one-tile object's turn (0x433f75), tick 20187.** Object 16's
tree 0x017f makes object 43, which turns to the facing of the frame's
stack object. The turn is the virtual function 0x4c of the object and
person classes (`Interpreter::turn_one`):

- A turn of 0 steps, or a turn step (data word 23) of 0, changes nothing.
- Otherwise the facing (data word 1) moves by the step times the turns,
  kept to 0 to 7.
- A seat (function 26) tells the tile ahead of its old facing, then the
  tile ahead of its new one, whether a seat faces it (0x433612).
- The walls beside the object are worked out again (0x434fac). The
  sides are turned by `(facing / -2) & 3`, and fences (styles 2, 12, 13
  and 14: 0x43517c, 0x467dec) do not count.
- The redraw (0x434101) is presentation and is left out.

**6. The placement's wall tests (0x476f5a), tick 24770.** The test reads
the object's wall rules (definition word `WALL_RULES`) against the walls
of its tile. They are turned to its facing (data word 1, from the caller
at 0x432acf) by `((-facing) & 7) >> 1`. The test runs in this order:

1. Diagonal walls: rule 0x40 without a diagonal refuses (code 0x17), and
   a diagonal without rule 0x80 refuses as a wall (code 7). So a tile
   with a diagonal wall refuses an object with no rules.
2. With no rules the place is then fine (`test ebx, ebx` at 0x476fd6).
3. Rule 0x20 needs two walls meeting at a corner (code 4 without), and
   rule 0x10 forbids that (code 5).
4. Each side, in this order: side 1 (need 0x1, forbid 0x100), side 0
   (0x8, 0x800), side 2 (0x2, 0x200), side 3 (0x4, 0x400). A forbidden
   wall that is there refuses as a wall. A needed one must be there and
   not be a fence. If a fence stands there the code is 0x16, else 6.

Before, only the diagonal check was ported, and any other rule failed
loudly. A diagonal refusal left no code (`Refusal::Unrecorded`), which
stopped a group's place test. Each refusal now has the original's code,
so that variant is gone. Codes 0x17, 4, 5 and 6 are kept as
`Refusal::Coded`. A group keeps its first tile's refusal unless a later
tile's code is 0x1e or 0x1f (0x422ba2 to 0x422bb3), and these codes are
neither, so a later wall refusal keeps the first. Only a later refusal
whose code may be 0x1e or 0x1f still fails loudly there.

**7. A person that took itself off removes itself and yields (0x58da12),
tick 26573.** In one update a person takes itself off the lot, which
asks for its animator's refresh, and then removes itself and yields.
The removal's own refresh is folded into the one already pending
(`Interpreter::refresh_again`), as the original's second call of
0x58da12 does. Any other removal while a refresh is pending still fails
loudly: of another object, with flag bit 0 (which goes on without
yielding, 0x451c1a), or in a synchronous run. (Flag bit 1 is another
thing: it sets the removal queue's mark, flag 0x1000, of section 8.)
An Expression between the two take-offs can set the head to follow a
target (person words 41 and 42), which the second refresh then measures
and the first did not. The test does that, and fails in the removal,
which shows that the second refresh is made.

**8. A person resets fully as it lets go (0x45e791 with an object), tick
26573.** When a person uses the object being let go of, or its running
action shows that object's icon, the original resets the person fully
(0x45e814 to 0x45eac4) before the filters (0x45eac7). Before, the port
went straight to the filters. Whether the objects the person used are
reset too is decided once, by object flag 0x1000 (0x45e7c8 to
0x45e7db). The flag is read from the object named, or from the person
itself when no object is named. The flag is the removal queue's mark
(`removal_skips_resets`), set by Remove Object Instance's flag bit 1
(0x451bc7, 0x442f60). The deletion (0x440eeb) changes only flag 0x40.
The full reset ends the running action: one with a target becomes the
last action, with flag 0x20 the Reset tree's result (0x45e6cd). It does
not empty the queue. Only a reset with no object named does that
(0x45ecc2). With an object, the filters at 0x45eac7 decide what goes.
The port used to assume the flag was clear. Honoring it changes the
state from before tick 9906: person 12 is reset while marked at about
update 6188 (tick count 94632). So the pins at ticks 9906 and 20186
change, and those from 2881 to 3912 do not.

**9. Look Towards mode 1 (0x45aab9 to 0x45ab66), tick 33097.** The person
turns towards the camera. Its position is turned into the view's frame,
within the 0x400 square (0x47bc1f with the view's rotation), moved
(16, 16) towards the viewer, and turned back by `(-rotation) & 3`. Then
it becomes the target, with the two coordinates swapped back to ours.
Data word 9 is cleared, and no stack object is read. 0x47bc1f with
`m = 0x3ff` maps (a, b) to (m - b, a), (m - a, m - b) and (b, m - a) for
rotations 1, 2 and 3.

**10. Create New Object Instance mode 0 and flag 0x8, tick 36311.**
Object 16's tree 0x0183 runs mode 0 with flags 0x9. Mode 0 (0x44e1e6)
puts the new object on the tile ahead of the running object: one tile
in its facing, by 0x42228a and 0x4222bc, whose steps are `forward`'s.
That is on its level, with no facing or room test, and the rest is mode
5's. Flag 0x8 (0x44e51e), tested after flag 0x1, refuses a tile where
any object stands. It is now also allowed in modes 5, 8 and 9.

**The pins.** Strict mode runs the whole day, 43,200 updates, with no
stop. The hashes at ticks 33 and 1103, from 2881 to 3912, and the
Newbies' do not change. Those at 9906 and 20186 change (section 8), and
those at 31478 and 43200 are new for this run. The older snapshot
formats' hashes for the ticks from 2881 on are dropped, since there is
no earlier state to compare them with. At the day turn the objects'
value comes to 12653, not the 12524 of the run before the fixes. The
objects are in other states by then, which is **unverified**.

Play mode with the snack order (`the_snack_order_pays_for_the_snack`)
changes the same way. At step 3603 the neighbors' people are 12 and 24,
not 374 and 375. The fire that run lit at step 13199 (Burn, #235) is
not lit any more. At step 15799 person 21 is off the lot, and its model
matrix there (0x45ee1d) is not ported. Burn keeps its synthetic tests,
but no real run lights a fire now.

The upstairs offset of section 4
agrees with #236 (`20261115-upstairs-person-takes-views-level-offset.md`),
which read the same vector, (0.5, 9.300511, 0.5), from the running
original's memory at all three zooms.
