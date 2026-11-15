# The player answers a dialog from the window

*Decided 2026-11-15.*

`sims_vm::boot::Lot` could only answer from a fixed list given at load,
so in the window the Goths' house had to say No to the Help System
before it opened. A lot can now wait instead (`Lot::wait_for_answers`).
When a tick shows a dialog with a choice and no answer is left, the
step does not fail. The lot goes back to how it was before the tick:
world, play-mode state, tick count and player. The dialog
(`Asking::shown`) and the world as the tick left it (`Asking::world`,
where the dialog's record is) are kept. Steps say `StepError::Asking`
and orders are refused until `Lot::answer` gives one of the dialog's
answers. The next step then runs the tick again from the start with
that answer queued. Until it has, orders are still refused: the lot is
as it was before the tick, and a listing or an order then would change
it where the original's never was (Sol's review found the window
listing a menu there). The simulation is deterministic, so the tick runs
as it would have with the answer listed at load. The test checks this:
the Goths' lot answered from the window gives the same world hash,
dialogs answered and sounds as `--answers 3`, tick for tick, through
tick 200.

Why the tick runs again rather than resuming: the dialog is shown in the
registry's pass at the end of the update (`0x4412d1`). That pass is
deep in the tick, and our interpreter cannot stop there and go on later.

- Strict boot never waits, so it is unchanged. `sims boot --answers 3`
  on the Goths' house to tick 1102 prints the same as main's, byte for
  byte. A lot that does not wait still halts at an unanswered dialog,
  as before.
- The cost is a copy of the world before every step of a waiting lot:
  0.1 to 0.4 ms on the Goths' house, measured with a step of 0.15 to
  0.6 ms. A tick at ultra speed has 5 ms here (`sims-game`'s pace).
- **Unverified:**
  - A sound the tick makes before the dialog is requested when the tick
    runs again, after the answer. The original presumably plays it
    before the dialog.
  - The original drew the house view many times while the Help System's
    dialog waited (`docs/roadmap.md`, the draw guard). We draw no frame
    of the simulation while waiting. A drawn frame writes animator state
    (`draw_frame`), so the frames are an input that may matter.
- The window (`sims-game`) always waits. A press on a button gives the
  answer the button stands for: 2, 3 or 4 for the first to the third
  (`Shown::answers`), and the tick runs again at once. While the dialog
  waits, the time owed to the lot is dropped, remainder and all, so the
  lot does not race to catch up after the answer.
  `--answers` still answers first. `--png` has nobody to answer, so for
  the Goths' house it still defaults to `3`.
