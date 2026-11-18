# A fire that burns a person resets whoever uses what burns

*Decided 2026-11-18.*

The play-mode snack order (`the_snack_order_pays_for_the_snack`) now
runs from step 15925 to step 16218. Three pieces of the original were
ported to get it there, all from its code. None is **verified**: no
recording has this fire.

**Removal at once through the lot's placement.** An object of one tile
that is not a person, removed at once (0x44123f) or reset as it is
deleted with no Reset tree (0x432055), leaves the lot by its virtual
function 0x50 with 1. For an ordinary object that is 0x433125, the
lot's placement (`Interpreter::take_off`): it is asked function 10, it
and what it holds move off the lot, and whatever held it is asked
function 9. That is the way now taken once the load has worked the
lot's rooms out. Before that, the placement cannot find the off-lot
place's room, so the narrow way the load was compared through
(`Machine::leave_lot`) is still taken. The strict boots go through
neither at the points they are pinned, and their hashes are unchanged.
Step 15925 removes a counter under a fire; the sink it holds goes too.

**Burn moves a person to its tile's centre** (0x44df25). The person's
position and level are read. It resets (virtual function 0x38 with 0)
and is put at the centre of the tile it stood on, at that level, with
the lot told (virtual function 0x58, called with the same arguments as
the fire's put). Then its virtual function 0x74 runs. The put is not
place-tested first, as the original does not test it.

**A person using what it is told to let go of resets fully** (0x45e791
with an object T, jumping to 0x45e814). This happens when P has a use
record naming T, or P's running action shows T's icon. P resets as it
does with no object named: its uses unwind one by one, and then P
resets. After that the filter of queue, last action, running action and
routes runs as for any person. Whether each used object is let go of by
everyone and reset is decided once, before the uses unwind, from object
flag 0x1000 (byte [ebp-0xd], 0x45e7c8): T's flag when T is given, else
P's own. Here that flag is `Object::removal_skips_resets`. Before this,
the reset with no object always reset what P used. It now reads P's own
flag, which only a person queued for removal with Remove Object
Instance's flag bit 1 can have.

**A reset of a person already in the middle of one is skipped** (object
flag 0x800, 0x45e7b2), where it used to fail loudly. Mortimer's last use
record names himself. When his reset ends that use, the object it
resets is Mortimer, who skips it.

**Where it stops.** At step 16218 fire 46 burns person 27, and person
27's reset makes Mortimer and person 12 let go of fire 41, which all
three use. Person 12 sits on a sofa. For a person inside an object, the
original takes it off the lot and tests and searches from where it is
then. It discards what it finds, takes the person off again, and puts it
at or near the lot's centre (0x45e8dd, 0x45e977). Taking a person off
the lot refreshes its animator only if the house view drew it, which is
not kept. A reset person stays on the lot afterwards, so that refresh
would have to be held until its animator's next tick, possibly in a later
update. That is not ported, and it fails loudly. The fault rolls the lot
back to before person 27's reset and resets fire 46.
