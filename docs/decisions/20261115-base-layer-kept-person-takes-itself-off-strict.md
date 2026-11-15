# The base layer kept; a person takes itself off; strict mode to tick 3432

*Decided 2026-11-15.*

**Unverified.** No capture covers any of it: everything here is from
tick 2837 on, and the Goth captures end at update 1000. Strict mode now
runs through tick 3432 (08:25:08, hash 359095d7…; fbb3d668… in format
25) and stops at tick 3433.

**Hashes.** Snapshot format 25 appends `World::base_layers`, and format
26 then `World::translated_bones`, so every pinned hash changes. The
format-25 encoding is a prefix of the format-26 one, and `boot.rs` and
`resize.rs` pin both (`format_25_bytes`): every format-25 hash this
entry first pinned (ticks 33, 1103, 2881 and 3432) still holds. The
format-24 encoding is a prefix of the format-25 one, and `boot.rs` pins
both (`format_24_bytes`): ticks 33 and 1103 keep
their format-24 hashes, and the state at tick 2836 is byte for byte the
one the previous commit (1bacd2d3) reaches (format 24, f74e31af…).
At tick 2881 the format-24 hash changes from eed688a0… to cdd5b72c…,
which is the head fix below; ticks up to 2836 are unchanged. Every
summary line of `sims boot … --ticks 2881` is unchanged. The K=1000
pins in `resize.rs` hold.

**The head looks at an object with its coordinates the right way
round** (0x59058f, 0x454f5c). The measurement passes native +0xf4 first,
which is our second lot coordinate, so the target is [3/16 ·
position[1], 0, 3/16 · position[0]]. #156 had them swapped. Track 4
found this and a review agreed.

**The base layer and the pose** (stop at tick 2882, 0x5992b3). The base
slot's playback is kept in `World::base_layers` and updated with the
skin's other layers in the animator's tail, timed events included. The
pose samples the layers by priority (0x5991fd): the base layer (-10)
first, then the foreground and the carry layer in the skin's order.
Each layer at weight 1 or more replaces what the bones held:

- A layer with nothing bound, no frames or no weight samples nothing.
- One below weight 1 blends (0x597c4c), which fails.
- The skin never puts the bones back at rest. A rotation on the way to
  a measured bone that no layer playing now writes is what an earlier
  layer left, which is not kept, so that fails.
- A translation is kept too: a motion with no positions leaves it as
  it was (0x598226, 0x598450). `World::translated_bones` (snapshot
  format 26) records, per person, every bone whose translation a layer
  the skin's update could sample has written since the person was made
  or loaded. A bone on the way to a measured bone that no layer playing
  now moves fails if it is in that record, and is at rest otherwise. The
  record counts every layer that could sample, without the update's
  "pose" argument (0x59820a) or the sample count, so it may hold more
  bones than the original moved; that only makes more cases fail. That
  a skin starts at rest at load is unverified. Of 7,950 clips, 7,947
  move ROOT and 7,577 PELVIS; SPINE is moved by 32.

A posture whose clip has no name starts nothing and leaves the slot as
it was (0x5904af, from the disassembly; retried next tick). Animate Sim
in the base slot now plays clips with timed events.

**Generic Sims Call 12** (stop at tick 2930, 0x44fd4b). Temp 0 becomes
minus the sum of the stack object's tile coordinates, turned by the
house view's rotation (0x434f58, the table 0x407704 builds), as a
16-bit word. A coordinate outside 0 to 63 at a rotation other than 0
fails, since the original reads outside its table.

The view's rotation (the scene at 0x6e5528, +0x84, passed at 0x44fdb8)
is the player's, not the lot's, but the simulation reads it, so it is an
input of `Lot` like an order: `Lot::set_view_rotation`, between steps.
Every interpreter `Lot` builds (load, step, orders, pie menu, selection)
passes it.
The game window sets it whenever its camera turns, from the panel or the
keys. **A recording of play must log each change** with the tick it came
before. The load starts it at the camera direction the house file saved
(HOUS, `boot::opening_rotation`), as the render track opens the view.
That the original's load turns the scene to it is not traced (its HOUS
read goes through 0x4b4263 and 0x4b42ba). The Goth and Newbie houses
both saved 0, and the comparison harness saw the Goth house open at
rotation 0, so strict boots of both run at 0. Which way the panel's two
buttons turn the original's scene is unverified.

**Generic Sims Call 2** (stop at tick 3356, 0x44fb51). It names the
running action's icon. #158 ported the same routine on main with the
same meaning, so the rebase keeps main's.

**A person takes itself off the lot in its own tick** (stop at tick
3356; Find Location mode 1 with the person as target). 0x433125
refreshes the person's animator before the registry hears of it
(0x4332e0, virtual function 0x54 with 1, 0x58da12), but only if animator
byte +0x109 is set. That byte is the house view's last draw test
(0x590b8b), which is presentation and is not kept. The refresh is a
small animator tick: the clock, the blend slot, the skin forced, the
place, and the head's step, measurement and restoration. Instead of
choosing a branch, both must agree. This is exactly what is checked:

1. At the take-off, the refresh is worked out on a copy of the lot as
   the take-off leaves it (after the holder's function 9 and the
   portal's tiles, before the registry, which does nothing for a
   person). The copy, with the person's animator, base layer and record
   of moved bones (`World::translated_bones`) put back, must encode byte
   for byte as the lot does, and the refresh must play no sound into the
   sound list. So the refresh writes nothing but those three, judged on
   the state it would really have read. A running head countdown (word 45) fails here, and so does a
   timed event that writes a hand or the censor word to a new value.
   The sound list does not hear the sounds of timed events, which are
   not modeled. So a timed event due in the refresh with a sound key
   ("sound", "selectedsound", "deselectedsound", "footstep") fails
   too: for a selected person "selectedsound" plays (0x59158c, 0x59175e,
   0x57a54c), and when the original plays nothing is not worked out.
   The lot goes on without the refresh.
2. Until the animator's tick, the person's thread may run only Sleep,
   Expression, Set Motive Change, Set To Next, and Change Suit with
   operand 1 set to 1. Anything else fails, naming 0x58da12. That these
   read no animator and no base layer is from their ports, not proven
   for the original.
3. At the animator's tick, the animator, its base layer and the record
   of moved bones must be those the take-off left (compared field by
   field).
4. The tick from the refreshed three, on the lot as it is then, must
   reach the same encoded world, and append the same sounds, as the
   tick alone.

What this does not cover: anything the refresh does that the port
leaves out (the draw state, the face, the pose, the head slot's
redraw). In the tick itself the sounds of timed events are still not
modeled, in both branches.

The tick alone is kept. The refresh then redraws what the head slot
holds (0x58dae5 to 0x58dc55): the container entry at begin + 0x48, our
slot 2, read when the person has more than two entries. For an ordinary
object (vtable 0x666d34) every write lands on its graphic ([obj+0x120],
from vtable +0x2c, 0x4215c3): the head bone's transform (animator +0xac,
0x5990e0, times animator +0xb0) through graphic +0xac, +0x64 and +0xb8,
and graphic +0x100 = the person's motive 3 as an integer (0x45d117 reads
+0x78c + 3 * 4; 0x636128 is the C runtime's `_ftol`, not a clock or a
random draw). Its +0x78 call is 0x51d372, `ret 4`. The animator tick's
tail (0x58f908 to 0x58fa7f) makes the same redraw every tick, which the
port already leaves out as presentation, so the take-off's redraw is
left out too; that is unverified. A person or a tile of a group in the
head slot fails, since their classes' functions (0x58dc08 to 0x58dc52)
are not read; this is checked before and after the take-off's
callbacks. A held hand (slot 1) fails too, though the refresh does not
read it, which can only add stops. A multi-tile person fails. The draw
state (0x434101) and the face (0x58faf6) are taken as presentation, as
in the tick; that is unverified. At tick 3356 person 21's head slot
holds its plumb bob (object 383, GUID 0x2F726FFF), an ordinary object;
person 21 then passes all four checks.

**Go To Routing Slot's interrupt tests** (stop at tick 3370):

- the door step (0x45afd0) and the asking step (0x45b475) fail with
  result 4, as step 1 does;
- the walk under way (0x45b2b7) takes result 4 to step 11, which stops
  the animator, plays `end_route` and fails (0x45b3c2).

These are on track 4's list; strict mode on the Goth house needed them,
so they are ported here. Only the door step has a synthetic test. The
other two use the same interrupt check and are tested only by the boot.

**Where it stops.** In strict mode, at tick 3433. Object 334 (tree
0x200d node 5) snaps a multi-tile object to a place where it does not
fit, and which object its group's test leaves as collided with is not
ported (0x454e82). The snack order (play mode) now runs without a caught
error until step 3603. There, object 354 (tree 0x1022 node 29) uses
Create New Object Instance mode 6 with flag 0x4, which is not ported.
With this and #158, strict mode on the Newbie house (`House07.iff`) runs
through tick 1472 (no recording to compare); at tick 1473 object 105
(tree 0x1017 node 0) gives up a route and needs the route-failure
feedback tree (0x45af2a), which is not ported.
