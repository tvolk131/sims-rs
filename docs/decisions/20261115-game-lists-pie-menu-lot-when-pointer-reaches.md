# The game lists the pie menu on the lot when the pointer reaches an object

*Decided 2026-11-15.*

When the pointer first reaches an object or a person on the lot, the
game lists the selected Sim's pie menu on it with `Lot::pie_menu`
(0x48d392), as the original lists it then (0x48c834); the listing keeps
what its check trees change, and its sounds are played with the steps'.
An object reaching a pointer that stands still counts too: each frame
drawn, the object under the pointer is looked up again. Nothing is
listed over the panel, a dialog or a menu, or once the lot has stopped.
A click opens that listing (or lists now, when the pointer did not move
onto the object first). (Superseded: a click lists again; see "A press
lists the pie menu again, and reaching an entry checks it again".) Only the entries whose check passes are shown,
in the original's order. An entry a check tree named with Add/Change
Action String (#140, as the fridge's meals are named) shows that name as
it is; any other, or one named with an empty string, is named by the
TTAs strings with the ID of the TTAB the simulation reads, by its menu
number, with "*" before it when the entry has flag 0x80, as the
original's getter does (0x41f167). Choosing one queues the very action
offered (`Lot::pick`). Where the simulation has not ported something the
listing needs (a trap of an unsupported kind, scope or primitive: an object
offers nothing; a version 0 table), the lot is unchanged, the menu shows every entry of the table
without checks, debug entries hidden, queued by menu number
(`Lot::queue_interaction`), and the note says why. Any other failure
shows no menu. The simulation reports every fault of a check tree as
unsupported and keeps the fault inside only as text, so for those the
game reads the inner trap from the text: a check tree taking its error
branch or running out of instructions shows no menu.

This runs menu checks on the shared simulation, where
`docs/vision.md` ("Other rules") runs them on a throwaway copy per
player. The original does it on the lot in single player, and #138 made
`Lot` do the same; the vision's rule is for multiplayer and is to be
settled there. Reopen when multiplayer lists menus, or when a recording
shows a listing at another moment.
