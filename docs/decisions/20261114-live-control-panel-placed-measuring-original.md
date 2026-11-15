# The live control panel, placed by measuring the original

*Decided 2026-11-14.*

`crates/sims-ui` draws the live-mode control panel from the original's
art (`UIGraphics`, ids from its control panel header) into a CPU frame.
The layout was taken from the old prototype
(`legacy/crates/sims-app/src/original_hud.rs`) and then checked against
a screenshot of the original at 800×600 (Goth house, live mode, Needs
open), by finding each bitmap in the screenshot.

What was checked:

- The cluster sits at the bottom left, 220 by 183. Every button's cell
  and position matched the screenshot exactly. Of its 28,244 pixels,
  255 differ, all on the translucent edge of the cluster or in the parts
  left out of the comparison (the clock line, the counter beside the
  mood face and the help button).
- The strip runs from x = 220 to the right edge, 100 high, its last
  column stretched to fill a wider window. Outside the meters, mood
  bars and portraits, 0 of 46,600 pixels differ.
- Portrait frames: 11 of 1,770 pixels differ, all under the activity
  icons, which are not drawn.
- The clock (font 8) and the funds (font 10) are drawn in green
  (565 value 0x2F2B) and match the screenshot's text. The Needs title,
  labels and meters match in place and color.

Three things in the prototype were wrong and are fixed here: the two
rotate buttons were swapped, the gadget sits 387 pixels from the right
edge (not 388), and at load the Live, normal speed, floor-down and
cutaway buttons show their selected cell and zoom-out its disabled cell.

The panel is placed against the bottom-left and bottom-right corners
at any window size, so a wider window shows more of the strip and the
lot. Only 800×600 was compared; how the original places the panel at
its larger sizes is **unverified**.

**Unverified or not drawn:**

- the counter beside the mood face (its source is not found; it may be
  an expansion pack's coins), and the help button;
- the activity icons over the portraits and the motive trend arrows;
- a family too large for one row: portraits wrap into a second row
  before the gadget (four columns at 800×600, so eight fit), and anyone
  past two rows is left out;
- which rotate button turns which way in the simulation's terms;
- the clock in the afternoon and the funds when negative.

Funds and the selected Sim come from the boot's household load (`sims
boot` steps 17 and 18), which the simulation does not offer as a
library call yet. The tests do the same two steps themselves; see the
roadmap section for what the simulation needs.

What would reopen it: a second screenshot (another window size, buy
mode, the afternoon) that disagrees.
