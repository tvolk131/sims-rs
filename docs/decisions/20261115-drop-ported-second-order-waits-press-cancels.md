# Drop is ported; a second order waits, and a press cancels either, in the window

*Decided 2026-11-15.*

**Drop** (opcode 5, 0x454430, from the general table at 0x45246a) is
ported from the original's code, as the inverse of Grab:
1. The object in my first container slot (+0x130, its second record) is
   looked up. Nothing there takes the false branch.
2. The place is my position plus one tile's step in my facing (data
   word 1; 0x42228a), on my level.
3. The object's place test runs on the ground there (virtual function
   0x54, no container). A refusal takes the false branch, and nothing
   changes.
4. The object is put there with the lot told (virtual function 0x58),
   and the true branch is taken.

- **Verified (Goth tests, play mode):**
  - The snack order, cancelled at step 900 while Mortimer eats
    (`a_cancelled_snack_is_put_down_in_front_of_the_eater`). His tree
    0x0139 (node 14) drops the snack at step 1009. He stands at (296,
    456) facing 6, and it goes to (296, 440) (corrected to (280, 456)
    by [20261115-seated-route-drop-pets-runs-route-failure](20261115-seated-route-drop-pets-runs-route-failure.md)). At step 1028 the
    cancelled action ends, and he is free. No error is caught.
  - Through the window
    (`a_second_order_waits_behind_the_first_and_a_press_cancels_either`):
    - The snack, then Water on plant 162: the water queues behind, its
      icon second, at x 82.
    - A press on the water's icon takes it off. The snack goes on to its
      end at step 1136.
    - A press on the snack's icon at step 899 (his sixth action, eating)
      cancels it. With the water queued, the cancel queues nothing of
      its own (0x45d7c0). He drops the snack at step 1009, and starts
      the water at step 1028.
  - A cancel at step 825, as he walks away from the fridge with the
    snack, stopped at the unported route stop below; since the route
    stop is ported, he stops the walk, carries the snack to (296, 456)
    and puts it down
    (`a_snack_cancelled_on_the_way_to_the_chair_stops_the_walk_and_is_put_down`).
- **A manual sweep, not a test:** the snack order cancelled at each
  step from 150 to 1136 (every tenth step, then every step near each
  edge), run 400 steps on. Cancels at steps 150 to 536, 543 to 590 and
  821 to 829 stop at 0x45b3eb; all the rest catch no error. Too slow to
  keep as a test (a full lot run per step).
- **Unverified:** no capture covers a Drop or a cancel. Which tree runs
  Drop and when is the scripts' own; only the primitive is ported.
- **Not ported (track 4):** cancelling while Mortimer walks, to the
  fridge or away from it with the snack, stops his thread at a route
  whose animator is still under way (0x45b3eb). Sol's review found the
  walk away (step 825), which the first sweep had missed by sampling
  steps 760 and 830.
