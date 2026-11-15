# A pie menu that offers nothing lists the other objects on the tile

*Decided 2026-11-15.*

When the listing on an object offers nothing, the original lists each
other object on that object's tile in turn (0x48d423, walking the tile
with 0x43f331 and 0x43f379, the links read again after each listing,
nothing outside the lot's size) and adds what they offer. Each listing
checks again whether the person is a pet. It leaves out an
object of type 8 and one whose definition word 91 lacks bit 0. Each of
those listings saves and restores the person's temps itself, and the
first listing's temps go back last. A menu that still offers nothing
is an empty menu, not a failure. The tile is the one the object's
position is on; for an object held in a slot, whether the original's
object +0xfc names the same tile is **unverified**. On the Goth house
283 objects now give an empty menu instead of failing, 75 offer
actions, and 19 fail in check trees on Add/Change Action String,
Manage Inventory operation 4, and scopes 24 and 12.
