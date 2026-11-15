# The selected Sim's actions show in the top left, and a press cancels one

*Decided 2026-11-15.*

The coordinator asked for the action queue to be drawn as in the
original, with the icons, their order, and cancel on a click. Track 6
had nothing for it in `sims-ui`. It is read from the original's code:
0x4d28df lays the icons out, 0x4ea31c collects them, 0x4ea203 builds
each, 0x4cabe0 and 0x4cb30c draw them, and 0x4cb126 handles a press,
which calls 0x45d5f8. The old prototype's
`legacy/crates/sims-vm/src/scheduler/clock/draw/actions` read the same
routines and agrees.

- **What shows** (`sims_ui::queue`). The running action comes first,
  then each queued action in order, each only when its icon names an
  object. The icons are 45 by 45, the first at (32, 32) and each next
  one 50 to the right. The picture is drawn first. `kIconFrame` goes
  over it: blue shows the picture and magenta cuts the corners. A queued
  action takes the grey cell. The running action takes the yellow cell
  once person data word 39 is not 0, and the grey one before that. In
  the snack, word 39 turns 1 at tick 615 (at the fridge), and the eating
  action the fridge hands over at tick 820 turns yellow at 837. An
  action with flag 0x100 has `kQueueCancelOverlay`, a red cross, over
  it. A person's picture is the household's portrait, cell 0. A person
  outside the household has none and shows as a dark grey (40, 40, 40)
  square with no frame.
- **A press** (`Lot::cancel`, `sims_vm::Interpreter::cancel`, 0x45d5f8).
  On the running action it clears person data word 33 and sets the
  action's flag 0x100. Unless the person would be interrupted anyway
  (0x45d7c0), it then queues the person's own interaction 33 at priority
  50 with no icon, which shows no icon. On a queued action it drops the
  action, and the target runs function 4 first. The VM already had that
  branch for routes (`Machine::release_action`), and the cancel uses it.
  The game plays `UI_queue_delete` (object -4) for a press that counts
  and `UI_error` for one that does not. A press on a pet's icon (person
  word 65 above 1) only plays `UI_error`. A second press on the same
  icon, or on an action already cancelled, does nothing.
- **Tested** on the Goths, through the game's input layer. The snack
  is queued from the fridge's pie and its icon shows at (32, 32) with
  the fridge's picture. A press takes it off with `UI_queue_delete`.
  Queued again, Mortimer takes it at tick 150. A press on it then shows
  the cross, and a second press does nothing. The VM test checks the
  same on `Lot` directly. It also checks the menu-33 action queued
  behind the running one, and that play mode then parks Mortimer at
  step 182. There he is walking to the fridge, and stopping a route
  whose animator is under way (0x45b3eb) is not ported. That is the
  VM's next gap for a cancel to finish.

**Not done.** An object's catalog picture (0x4ccaa0, which the original
renders from the object's sprites) is not made yet. An object's icon
shows its frame over the dark grey of a missing picture.
The icons do not slide to their places over 500 ms (0x4cb1df), and a
new icon does not start at the pie. A press during a slide, which the
original ignores, is taken here. The action's name as a tooltip is left
out, because the queue keeps no names.
**Unverified:** that the sound player finds `UI_queue_delete` and
`UI_error` for object -4 as the original's does. No capture compares the
drawn icons yet.

**Ours, from Sol's review.** A press on the status band across the top
is the band's, even where a wrapped note covers an icon. A cancel that
never reaches the simulation (the lot refuses it, or has stopped) leaves
the icon to take another press. In the original a cancel cannot fail,
and it has no such band.
