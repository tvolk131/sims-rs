# Deselecting a Sim is ported but for its animator's refresh; play mode switches Sims

*Decided 2026-11-15.*

Selecting a Sim (0x445449) now ports the deselection of the Sim selected
before, read from the original's code (0x445449, and 0x44664c from
0x4468c3 for a Sim without the mark), with the old prototype as a guide
(legacy/crates/sims-vm/src/selection.rs and selection_indicators.rs).
The old Sim's mark is taken off. With nobody selected, the first
indicator is taken off the lot (its virtual function 0x50 with 1, as
`Interpreter::take_off` takes an indicator), the Sim's animator is
refreshed, and the second indicator is taken off too. A Sim with person
word 81 at 10 or more would then take a Superstar star marker
(0x446c15); that fails loudly, whatever the expansion mask, as the
placement of a person from outside does. The redraw (0x434101 with 0)
is taken to be presentation (**unverified**). Then the new Sim is
selected as from nobody, as before.

The animator's refresh (virtual function 0x54 with 0, 0x58da12) is not
ported. With 0 it always runs: it reads the clock again, which between
ticks has moved on from the animator's, brings the skin up to date,
places the animator, and runs the head's step, which can count person
word 45 down and write word 42. So in strict mode (`Interpreter::select`,
`Lot::select`) deselecting a Sim while the lot has indicators fails
loudly there, and a selection that fails changes nothing. Play mode
(`Lot::select_play`, `Refresh::Skip`) leaves the refresh out, so the
deselected Sim's animator, head and words do not move as the original's
would; such a lot is not one to compare with the original. The game
window selects with `Lot::select_play` in play mode and `Lot::select`
in strict mode, where switching Sims therefore fails with a note.
`sims-game` carries out the panel's `Command::Select` this way, and the
panel, the motives shown and the pie menus follow the new Sim. Which people the original's
portrait press lets the player select, and whether it does more than
0x445449 (the camera following the Sim, for one), is not read: the
panel shows only the household's portraits.

**Unverified**: no recording selects a Sim. Strict `sims boot` of the
Goth house is unchanged (1102 ticks, output identical); a removal of the
selected Sim (0x44123f) reaches the new path, which still fails there
(now on the refresh rather than on "deselecting is not ported"), and the
Goth run removes no selected Sim. Reopen when 0x58da12 is ported, or
when a recording of play selects a Sim.
