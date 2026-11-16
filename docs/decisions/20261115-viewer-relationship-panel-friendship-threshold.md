# The viewer's relationship panel reads the friendship threshold from the program

*Decided 2026-11-15.*

`sims-viewer --hud` now fills the relationship panel as the window does
(`Snapshot::with_relations`, read only while the panel is open, with
`Acquaintances::read`). It loads the faces of everyone the world's
neighborhood records hold, not only the household.

The viewer may show a snapshot or a house file it did not boot, so it
has no lot's program to ask for the smileys' friendship threshold. It
links the global file alone (`Program::link` with no types), which holds
the tuning, and reads `Program::autonomy_constants` as the lot does. An
earlier version of this change read the tuning table in `sims-ui` with a
second copy of that code; a review found two ways the copies differed
(which chunk wins, and the `__ftol` model), so the copy was dropped.

Moving the pointer onto, off or between entries redraws the panel
(`hud::lit_relation`): an entry is drawn lit under the pointer though it
takes no press, so the hover check that follows pressable items missed
it. The window's hover check uses it too.

- **Verified**: on the install, the global file alone and the booted
  Goth lot give the same threshold, 50 (`crates/sims-game/tests/tuning.rs`).
- **Unverified**: the viewer's panel has not been compared with the
  original on screen. It shows the same snapshot the window's panel does.
