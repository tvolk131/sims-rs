# The panel follows the lot; a portrait under the pointer shows a first name

*Decided 2026-11-15.*

The coordinator asked for the panel to be wired to live state: the
selected Sim's motives, the clock, the household's money and the
selected Sim. Most of it already was. `Hud::draw` lays the panel out
from `Snapshot::read(world)` every frame, and the window draws a frame
after every batch of ticks. What is new:

- **A test that it follows the lot.** The Goth test that queues Have A
  Snack from the fridge's pie now runs 850 ticks on. The panel's text
  has to show the world's clock and funds before and after. The funds
  show $2,542 and then $2,537, after Mortimer pays 5 at tick 820. The
  clock moves, and the needs panel's first meter has to be the selected
  Sim's hunger (motive 7) both times. `Game::panel()` gives the items
  last laid out.
- **The selected Sim.** The strip already lights the selected Sim's
  portrait (the template's selected cell). The original's live
  screen shows nothing else of the selected Sim's own. The icon in the
  top-left corner of the original's screenshots is the action queue
  (a bed, and a Sim's head for a social interaction), not the selected
  Sim's portrait. That queue is not drawn yet.
- **The name.** A person's first name is the first string, in US
  English, of the catalog strings (`CTSS`) that word 41 of the type's
  `OBJD` names. Mortimer's file has 2000 there, and `CTSS` 2000 starts
  with "Mortimer". Word 19 of the same file is 200, its body strings, so
  the count of words is right. `Faces` reads the name with the face.
  While the pointer is on a portrait, the panel draws that person's name
  in a box over the strip, above the portrait. The box takes no presses
  and does not cover the lot.
  **Unverified, and ours:** where and how the original shows a name.
  We believe it shows one on hovering a portrait, but no capture has
  been compared. The box's look (the needs panel's label color on
  black, font 8) is ours. Reopen when the harness can hover a portrait.
  The box is as tall as the font's line with 2 pixels above and below,
  so descenders stay inside it, and its bottom is 2 pixels above the
  strip. It is kept inside the frame: a name wider than the frame is cut
  short. The name goes when the pointer leaves the window. (Sol's review
  of the first version found the box overflowing at extreme rects,
  descenders below it, the box off the top of a short window, and the
  name staying after the pointer left.)
