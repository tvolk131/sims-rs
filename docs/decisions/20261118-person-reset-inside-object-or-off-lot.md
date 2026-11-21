# Reset placement for a person in an object or off the lot waits on the animator

*Decided 2026-11-18.*

The original's call order was read from its code. It is **unverified**
against a recording. The strict Goth day does not reach these branches.

The full reset first resets the animator and reloads the appearance
(0x45e873 to 0x45e893), erases the route contexts (0x45e8aa), and clears
the headlines (0x45e8af). It then captures the person's place (0x45e8cd,
0x43265e). The rest of placement runs before the person's thread is
restarted and the Reset tree runs.

1. A place with a container takes the person off the lot with notification
   (0x45e8dd, virtual function 0x50, 0x433125). The reset reads the position
   and level after the call and tests that point without a container
   (0x45e909, virtual function 0x54, 0x433311). On refusal it runs the
   spiral search at 0x4347f0, allowing occupied tiles to reach the place
   test and starting in the search origin's room (0x45e910 to 0x45e943).
   On failure it repeats the search without the room rule (0x45e952).
   The search can run overlap callbacks through the place test. Its
   returned point is unused on every branch.
2. Object flag 0x40 skips the rest of placement (0x45e95d). Otherwise
   0x432595 checks whether the person is on the lot using the lot's own
   bounds: its place capture (0x4325b5) calls 0x422632 with the lot
   (0x6e552c), which tests each tile against 1 and the lot's size (+0x10)
   less 1. `placement::capture`'s own flag still uses the arrays' 64 tiles
   (`placement::on_lot`); that difference is left for a follow-up. If off the lot, it takes the person off with notification
   (0x45e977). It reads the level again after take-off, normally level 1,
   and tests the centre of tile `(size / 2, size / 2)` (0x45e9e9). On
   refusal it runs the same two searches from that centre (0x45ea1d,
   0x45ea33). A failed search leaves the destination untouched (0x434c3a,
   0x434c4d). The reset puts the person at the resulting point even when
   both searches fail and the point is still the centre (0x45ea43).
   The person's virtual function 0x88 (0x45e4e9) puts it with notification
   and notes the new room, as `Interpreter::put_person` does. Virtual
   function 0x74 (0x45e519, `Interpreter::refresh_person`, called at
   0x45ea4d) then unsets the animator's position and sets person word 64
   from the pool test. Only this branch calls it: the deletion mark and an
   on-lot person jump to 0x45ea52 (0x45e962, 0x45e971), past the call, so
   the port's on-lot and deletion resets rightly leave it out.

The existing `fits` and `search` should cover the placement tests and
both searches (**unverified**: no test runs them with these parameters).
The search record has an explicit start and level, direction
−1, no random start and no ground-flag rule (0x4347d1, 0x45e910,
0x45e9f0). Both searches allow occupied tiles. `Machine::leave_lot`
handles only a narrow pre-rooms take-off for ordinary objects already
off the lot. The person needs `Interpreter::take_off`'s one-tile path,
including callbacks, the animator refresh before notifying the registry
(0x4332ed), and notification.

**Open.** The take-off refresh (0x58da12 with 1) does work only if animator
byte +0x109 is set (0x58da2c). The house view's draw writes the byte
(0x590b8b), and the animator's tick can write it too. The port has no
value for it. The later 0x45e519 unsets only the animator's position and
direction (0x5893a9, 0x589361). The earlier refresh can still have changed
the clocks, the head and the animator's destination (0x58da41, 0x590768,
0x592132). The second take-off needs to use the refreshed branch if the
first took it. The refresh can also change person words such as the
head countdown, which `refresh_at_take_off` already refuses.

`Pending` holds the two animator states. `Refreshing` belongs to one
person's tick, and `taken_off` belongs to one scheduler pass and is
consumed for a person queued for removal. A reset can run after that
person's tick, in a partial pass that skips it, or outside a pass during
a visit. It can leave the person on the lot with both states still
possible. Porting that case needs a per-person pending refresh that
survives a pass and is used for a person who stays on the lot. The
placement calls and callbacks between the take-offs, and any intervening
animator reads or writes before the next tick, would have to preserve
both states or fail if they could tell them apart. The next animator
tick would have to compare the two outcomes at its own clock. The
current take-off code checks this within one pass. It cannot carry the
refresh through the reset's later calls or into another update.

Both reset branches therefore still fail with `Trap::Unsupported`,
naming the reset and refresh addresses. The off-lot guard now uses
0x432595's lot bounds, as the original does. Synthetic tests check that
a seated person stays in its slot and an off-lot person stays off the
lot on refusal, and that neither runs a callback or changes its
animator. The deletion test checks that a person already off the lot
runs its last Reset tree and is freed without placement callbacks.
