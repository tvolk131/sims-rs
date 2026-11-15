# A person is taken off the lot and deleted; Snap turns a slot's offset; Create mode 6 takes flag 0x20

*Decided 2026-11-15.*

**Unverified.** No capture covers any of it. The Goth captures end at
update 1000; the removal is at tick 1104, the turned Snap at 2515 and
the new Create at 2703. Strict mode's output for ticks 1 to 1102 is
unchanged line for line, hash 9b09ff4f… included, and the K=1000 pins
are unchanged.

**Who leaves.** Person 259 is the traveling salesman
(`npc_travelingsalesman`, GUID 0x44A86A6C, neighborhood record 301).
Object 382 makes him at the pedestrian portal in tick 6. At tick 1104 he
walks to the portal and removes himself (global tree 0x0176: Go To
Relative Position, then Remove Object Instance on himself, node 27).

**Taking a person off the lot** (virtual function 0x50 with 1, 0x433125,
shared by the person's vtable 0x667378 with an object's) is the one-tile
take-off an object gets, plus one call: the person's animator (virtual
function 0x2c, 0x4594de) is refreshed (its function 0x54 with 1,
0x58da12). The refresh returns at once unless animator byte +0x109 is
set (0x58da2c), which is whether the house view's last draw found the
person on screen (0x590b8b). That is presentation and not kept, and the
animator's tick writes it too. It is not always set for a drawn person:
in the captures the salesman has 1 at update 900 and 0 at updates 950
to 1000. We take the "not drawn" branch, which is **unverified**. When
it is set, the head's step (0x590768) advances the head's clock and
counts person word 45 down, which a script can read. So only the cases
where that cannot be read first are taken:

- Remove Object Instance takes a person only when it removes itself and
  yields (not with flag bit 0, not in a synchronous tree). Its own tick
  follows at once and runs the head's step at the same clock, and the
  removal at the end of the update destroys it.
- Removal at once (0x44123f, which the end of the update also runs for
  the queued person, refreshing it a second time) deletes the person
  right after, so only the deletion's last reset could read what the
  refresh wrote. That it does not is **unverified**.

Every other way of taking a person off fails loudly, naming 0x58da12.
Whether the head's step done twice at one clock (refresh, then tick)
counts word 45 down once or twice is not read; a reader before the end
of the update is the only way to tell. What could still differ is also
read by a head looking at the queued person (0x59293d) or by a person
still queued after a partial pass, and both fail loudly.

**Deleting a person** (0x440eeb) runs in this order:

1. It gives up the interface events (0x444bc8).
2. A selected person would be deselected (0x445449), which is not
   ported and fails.
3. Every person lets go of it (0x444306).
4. Its last reset runs (virtual function 0x38, 0x45e791) with the
   deletion mark (object flag 0x40). The mark skips the move back onto
   the lot (0x45e95d). The reset also erases the person's route
   contexts (0x45e8aa, 0x45fad7) and keeps its path. That erase holds
   for every person reset, which before failed on any context.
5. Each object it holds is removed at once (0x441196).
6. Its sounds stop, and it leaves the object list and its type's
   instance list (0x44383c). The person's destructor (0x459511) frees
   the route, the animator, the uses, the contexts, the path and the
   queue. So the World drops its animator, route, tick state and unknown
   data words, and the lot's list of people loses it.

The deletion writes no neighborhood record, relationship, family or
room. Other objects' frames and relationships that name the ID keep it,
as in the original. The animator's reset now stops a route
(0x5892dd → 0x589e2d, result to 0 at 0x5892e4). For a result other than
1 or -1, the destination becomes the unset position (0x589e5d). A
removal before the load has worked out the lot's rooms, a person with
a routing frame count, and a person on several tiles all fail loudly.
A removed person's room keeps its occupied mark, which is the
original's behaviour. Create mode 8's removal of a new person that does
not fit now goes through the same path and takes the false branch.

**Snap with an offset** (0x454b93 to 0x454c54) works in world units:

1. The object's position becomes `[3/16 x, 0, 3/16 y]` (0x454f5c).
2. The slot's offset becomes `[3/16 o1, 0, 3/16 o0]` (0x454ecc).
3. The offset is turned by the matrix of the raw facing word (0x455070,
   0x5c3a20 axis 1; 0x5c4510 with every term in the original's order).
4. Each turned coordinate is added to the position and stored.
5. The sums are rounded back half away from zero (0x454fb9).

The cosine and sine come from libm, with the caveat `Ext::cos` already
gives. Routing slots use the same helpers. The bound on how far out a
position may be is gone, since the round trip is now computed.

**Create New Object Instance mode 6 with flag 0x20** turns the new
object to the stack object's facing with bit 0 cleared (0x44e640). This
is mode 8's code, shared. A turn (0x433f75) still fails. Mode 6 never
places the object (0x44e72c). Mode 6 making a person now fails loudly
at 0x44e60e, where before it would have skipped the person's traits.

**On Goth.** Strict mode runs through tick 2702, hash 8b89f19d….

- At tick 1104 the salesman is removed (hash at 1103:
  1849f00c…).
- At tick 2515 object 21's Snap (tree 0x2018) turns its slot's offset.
- At tick 2703 the carpool (object 8, tree 0x1017 node 4) makes the
  school bus (GUID 0x50AC15BF), a multi-tile object, and the factory
  for multi-tile objects is not ported. That is big, so strict mode
  stops there.

The snack order removes 259 at step 1104 and stops at the same school
bus at step 2703 (tick 91146).

After the Goth social (the pie-menu run in `boot.rs`), the salesman
leaves at tick 597 while his animator plays "A2O-door-knock". The reset
that deletes him would remove that clip, which can fade it out
(0x592878), and that run stops there.
