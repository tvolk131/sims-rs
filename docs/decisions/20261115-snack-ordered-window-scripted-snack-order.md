# A snack ordered in the window is the scripted snack order

*Decided 2026-11-15.*

In play mode, the window's order path is the one the original takes:
the pointer reaching the fridge lists its pie (`Lot::pie_menu`, which
runs the check trees), a press opens it, and a press on Have A Snack
queues the action the listing offered (`Lot::pick`). The scripted snack
order (sims-vm's `the_snack_order_pays_for_the_snack`) queues the same
action by menu number (`Lot::queue_interaction`) without listing first.

- **Verified (a Goth test, `the_snack_ordered_in_the_window_is_the_scripted_order`):**
  the action queued at step 51 is the same, field for field. From step
  150, when Mortimer takes the order, the two lots hash the same at
  every step through step 1136, when the snack ends. Play mode catches no
  error. Mortimer pays 5 at step 820. The test also checks that the
  chosen entry carries the action the check trees listed (not the
  fallback's menu number), and that the lots differ at every step
  before 150, so it fails if the window skips the listing (from Sol's
  review, which found the test passed with the fridge forced onto the
  fallback menu).
- **The difference before step 150:** the listing's check trees run as
  Mortimer. They leave his last result and his saved stack frame's
  words, and nothing else (compared field by field at step 51). His
  first tree at step 150 rewrites both. The original lists the menu
  when the pointer reaches an object (0x48c834), so the window's lot is
  the more faithful of the two. Whether the original's listing leaves
  the same words in Mortimer's frame is **unverified**: no capture
  covers a listing.
- The test finds the fridge on screen without moving the pointer, since
  each new object the pointer reaches lists a menu and changes the lot.
  A player's pointer that crossed other objects first would list their
  menus too.
