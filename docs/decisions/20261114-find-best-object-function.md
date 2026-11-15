# Find Best Object for Function

*Decided 2026-11-14.*

Tick 417 of the Goth house stopped on Find Best Object for Function
(opcode 14): person 21's bookcase tree (0x1014) asks for the best object
for function 8. The original's handler is `0x44f2bf`, reached through
the primitive table at `0x45246a`.

What was decided:

- **It is ported whole** (`find_best_object.rs`), every function and
  flag of the handler, not only function 8: the table at `0x44f641`
  (callback and score attribute per function), the room flags, the
  check trees, the origin a variable names, function 11's halved
  distances and function 15's category walk. Functions 4, 5 and 9 give
  false, as the original's table does.
- **The walk is registration order.** It follows the list at
  `[lot+0x30]` through each object's +0xe0, the list Set To Next mode 2
  walks, which is `World::order`.
- **The weight is a setting.** The 3 that multiplies the distance is
  the global tuning's "functional distance attenuation" (`0x6c4394`,
  written once at `0x4592c4` with a default of 3), read with the other
  autonomy settings (`autonomy::Constants::distance_attenuation`).
- **An entry's check tree is our callback `condition`.** The original
  reads offset 0 of an entry as the action, as its fallback from the
  definition's action words shows (`0x4561e0`), and offset 2 as the
  check tree. In the game's files the second field we parse is the long
  tree and the first the short "CT - ..." ones, so `condition` is the
  check tree.
- **A check tree that changes the object list traps** `Unsupported`,
  whatever it gives: the original walks the live list, and we walk a
  copy.
- **An unordered score counts as 1**, as the original's `fcom` and
  `jae` (`0x44f5ef`) give, and never replaces the best (`jbe`).
- **Function 8's person test** reads person data word 65 (+0x60e), the
  word Idle For Input reads; the original casts the running object to
  its person class first, and a non-person gives false.

**Unverified**: the primitive against a capture of its own. It picks
object 174 in tick 417, as the fourth capture shows; the tick then
stops on Animate Sim's carry slot (`0x589571`), so nothing after the
pick is compared yet.

What would reopen it: a tick whose pick differs, or a check tree that
edits the object list.
