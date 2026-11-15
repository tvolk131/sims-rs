# The lot lists pie menus and takes picks on the real lot

*Decided 2026-11-15.*

`Lot::pie_menu(person, object)` and `Lot::pick(person, action)` run the
interpreter's listing and pick on the lot itself, not on a copy, because
the original's listing runs check trees that change the lot. A recording
of play has to note each listing, at the moment the original makes it:
when the cursor first reaches an object (0x48c834), not on the click. A
listing or pick that fails on something not ported changes nothing and
does not halt the lot, as for `Lot::queue_interaction`. The listing
returns the sound requests its check trees made with the actions.

On the Goth house the selected Sim can list and queue entries on several
objects. Every entry on another person still fails on a queued action
that runs at once (0x45d240). Reopen when a recording shows a listing at
another moment, such as on a click.
