# The game draws its pie menu with `sims_ui::pie`

*Decided 2026-11-15.*

The window's pie menu is now `sims_ui::pie::Pie` (#157), the port of the
original's menu window and object picker, in place of the plain list
`sims-game` drew. The game hands it the entry names in the listing's
order and keeps it while the menu is open; the pie groups a name with a
`/` into a submenu and shows at most 12 entries, and a choice is the
entry's index in the listing, which the game queues as before. A press
on a submenu's entry, or back out of one, keeps the menu open; one
elsewhere closes it. The pie is laid out as it opens and again after
each press that keeps it, so a second press finds what the first
opened. As `sims_ui::pie` says the original does, the window moves the
pointer to the middle of a submenu that opens, and of a menu that had
to move to stay on the screen; where the platform refuses to move the
pointer, it stays.

**Verified:** unit tests choose an entry, open a submenu and choose
from it, press twice with no drawing between, move the pointer for a
menu moved on the screen, and close the menu; on the Goth house a test
clicks the fridge, finds Have A Snack on its pie and presses it, and
Have A Snack (its menu number on the fridge) is queued for Mortimer.
Moving the real pointer is not tested; no test opens a window. The placement itself is `sims-ui`'s, checked there
against the running original.

**Not verified or not done:** the middle of the pie shows the face
bitmap, which is all colour key, without the selected Sim's head; the
original renders the head into it live, which needs `sims-people`. The
game takes a press when the button goes down and never draws a label
in the held colour (`pressed` is always false); which of down or up the
original's picker answers on is not checked here. Reopen when a
screenshot of an open pie disagrees, or when the head is rendered.
