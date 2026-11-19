# The plumb bob is picked as itself, and its pie is what sets Mortimer's word 50

*Decided 2026-11-18.*

In the replayed snack order (`goth-snack-order-20261115`), Mortimer's
data word 50 is 1 in the recording from update 622 to 871 and 0 in ours
(`20261115-replay-draws-recording-frames-queue-listing.md`, unverified).
The write watch recording (`goth-snack-d50-watch-20261115`, the same
order with a held press and a write watch on that word) answers which
code writes it.

- **The writer.** The settling write (0 to 1, between updates, eip
  0x452e8e in the Expression primitive) comes from a pie listing for the
  selected Sim. The frames saved with the hit, read past the six the
  frame-pointer chain gave, are:
  - the check (0x458319), from the pie's action maker (0x458705);
  - the listing (0x45859c, through the switch at 0x458583);
  - the house view's change of the object under the pointer
    (0x48c834), called at 0x48ca2c;
  - the frame's draw (0x4d2062).

  The arguments saved with that call are object 383 and the point
  (200, 123): the plumb bob (GUID 0x2F726FFF, `sims_people::bob`), under
  the pointer where the snack's entry had been pressed. Earlier hits in
  the same recording give the fridge's counter tile (119 at (176, 222))
  and the fridge (118 at (175, 198) and (176, 198)), the hovers the
  harness logged. In both recordings the bob's storage byte 276 takes
  0x08 at the same update as the word (623 in the order, 685 in the watch
  recording), so the order's listing is on the bob too.
- **How the bob comes under the pointer** (read from the code). The
  pointer pick at 0x48c9be takes the graphic noted as under the pointer
  (0x58462f, the global 0x6f6bc4), casts it to an object, and lists that
  object's id (+0xf0); with none, it lists the sprite pick's object. A 3D
  graphic notes itself while it is drawn:
  - the bob's draw (0x5863eb to 0x586455) and a person's (0x590ebd to
    0x590f84) read the depth buffer (the main surface's attached surface,
    0x4b0232) at the pointer before and after drawing the mesh;
  - if the value changed, the graphic is the one under the pointer.

  The pointer is `GetCursorPos` made client-relative. The bob draws
  after the person, so where it covers them it is the bob that is found.
- **The port.** The bob's pixels now name the bob (`People::draw_on_lot`).
  Before, they named the person holding it, which the plumb bob note left
  unverified. So the window's pointer on the bob finds object 383 unless
  a person is shown within the pick's reach of the pixel
  (`crate::pick` finds people first, `pick::REACH`; where the bob
  overlaps someone nearby, as after some camera turns, the person is still
  found, which is **unverified** against the original's draw-order rule).
  The pick lists the bob's pie for the selected Sim, as the original
  does. The listing offers nothing. Of Mortimer's object words, only word 50
  changes, to 1. His stack frames (tree, node, locals) and last result
  stay as they were. His frame storage grows, as any check's does. This is the
  recording's change at 622 for the words it compares
  (`the_pointer_on_the_plumb_bob_lists_the_bobs_menu`).
- **Unverified: where the original's bob was at 622.** The recording's
  pointer stays at (200, 123) from the choice at 202. Our window shows
  the lot 97 pixels right of and 46 below the recording's. That offset
  correlates the recording's screenshot at update 204 with our frame to
  within 2.3 levels per pixel. The bob over Mortimer at the door
  (update 64) matches the screenshot's to a pixel. But as Mortimer walks to
  the fridge, our bob passes x 200 (in the recording's pixels) at about
  y 150, about 26 pixels below the pointer. At 621 it is at
  (209..213, 139..150). So the replay still leaves the word at 0 until
  872. People are found at the pointer as expected: person 18 at (400, 274)
  in the social order's hover, a few pixels below her drawn feet, and not
  at (400, 286). The bob's own pick point is what does not fit. A
  recording with screenshots during the wait (updates 560 to 700) will
  show where the original draws the bob then.
