# A group snaps where it does not fit and is deleted; strict mode to tick 3602

*Decided 2026-11-15.*

**Unverified.** No capture covers any of it: the Goth captures end at
update 1000. Strict mode now runs through tick 3602 (08:30:48, hash
07ce1fc9…) and stops at tick 3603, where object 354 uses Create New
Object Instance mode 6 with flag 0x4: the stop the snack order already
reached, which track 4 has. The snapshot format does not change, and
every summary line of `sims boot … --ticks 3432` and every pinned hash
up to tick 3432 is unchanged.

**Snap of a multi-tile object that does not fit** (stop at tick 3433,
0x454e82). The group's place test (0x422abb) goes on through every tile
after one refuses (0x422c08), and each tile's collision writes the
object collided with (0x6e4ad0, 0x433b42) over the last; nothing else in
the test writes or clears it. So Snap's word 54 is the last object any
tile's test wrote, whichever tile's code the group keeps
(`Interpreter::place_test_collided`), and a group's
`Refusal::Collision` names that object too. At tick 3433 object 334,
the lead of the car junk (`CarJunk.iff`, GUID 0x4B748E3B, 18 tiles,
made with the school bus at tick 2703), snaps to a place off the lot:
word 52 becomes 2 and word 54 0.

**Find Location For mode 1 and Remove Object Instance on a tile of a
group** (stop at tick 3434, 0x44b52a). Both first ask whether the
tile's type names function 8 (0x42344e, the first word of callback
entry 8). If it does, the tile leaves its group first (0x42346a; for
the removal, the tile goes alone, 0x442def), which is not ported and
fails loudly. If not, Find Location takes the whole group off
(0x422d86: every tile is asked function 10, then every tile leaves),
and the removal queue takes the group's lead tile in place of the tile
(0x442d9d to 0x442e38). At tick 3434 object 334's tree 0x2000 does
both on itself (node 33, then node 2, which yields).

**A group is deleted** (0x440f37). At the end of the update the
registry's drain (0x442447 to 0x4424c1) makes the same test of
function 8 for a tile of a group, then removes it at once (0x44123f):
its function 0x50 takes the group off again, and the deletion
(0x440eeb with 0) goes to the lead (0x440f6f), which marks every tile
for deletion (flag 0x40, not kept) and deletes each tile in turn along
the group's list (0x440fe3, with 1), reading the next tile first. Each
tile's reset (0x431fbe) runs its Reset tree if its type has one, or else
takes what is left of its group off once more (function 0x50 with 1,
0x432055), so every remaining tile is asked function 10 again. Each
tile's destructor (0x42283a) then takes it out of its group (0x4228d6):
the lead hands the group to the next tile, which every later tile takes
as its lead. So the take-offs ask function 10 of n, n, n − 1, …, 1
tiles. None of the car junk's tiles has a Reset tree, function 9 or
function 10, so at tick 3434 those take-offs run no tree; all 18 tiles
go.

**A deletion's reset stops the object's sounds.** Every path of an
object's reset (0x431fbe) with argument 0, which the deletion
(0x4410a5), error recovery (0x439acc) and the load's visit (0x431f68)
all pass, meets at 0x43208b and asks the sound system to stop the
object's sounds (0x4320b4); a nonzero argument jumps straight to the
exit (0x431fc7 to 0x4320d4); the deletion's mark (flag
0x40) only spares the rebuild (0x432063). The port had left that stop
out for a deletion, so a deleted object now notes `SoundEvent::Stop`
twice, from its reset and from its destructor (0x4323c9). Sound
requests are not in the snapshot, so no hash changes. Sol found this.

A tile of a group removed before the load has worked the rooms out
(the load's clear) fails loudly: its take-off needs them.

**Review gaps closed.** Sol's review of #170 noted three optional
regressions. Two are added: a take-off callback that puts something in
the person's hand is caught by the check after the callbacks, and a
tile of a group in the head slot fails (0x58dc08). Selection at a
nonzero view rotation is not: it needs a loaded lot whose indicator
trees read Generic Sims Call routine 12.
