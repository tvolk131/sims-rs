# The plumb bob is the original's mesh, coloured and lit as the original does

*Decided 2026-11-15.*

The selected person's plumb bob is now drawn (`sims_people::bob`, from
`People::draw`). Everything below was read from the original's code in
a memory image, except where it says otherwise.

- **What it is.** An ordinary object (`PlumbBob.iff`, GUID 0x2F726FFF)
  in the person's head slot (their container slot 2), with no sprites.
  Its object string 200 names the skeleton `root` and the suit
  `head-arrow,ROOT=white`. The suit's mesh, `xskin-head-arrow-ROOT-ARROW`,
  is in `Animation.far`, 2.1 units long on its z axis. No
  `root-skeleton.cmx.bcf` or `white.bmp` exists, so we make a one-bone
  skeleton and draw the mesh white.
- **Where.** The head slot's redraw (0x58dbd8 to 0x58dc03) gives the
  bob's graphic the `HEAD` bone's position times the person's
  placement, without the bone's own turn. The draw (0x585dc5) sets the
  root bone at (0, 3, 0), turned by the original's Euler routine
  (0x5c4bb0, run in an emulator to fix its order) a quarter turn about
  x and the spin about z. So the bob stands upright from three units
  over the head and spins about its length.
- **Spin.** The game's clock in milliseconds (0x6e54e0) × 0.001 × π, or
  half a turn a second. We have no such clock, so the game window passes
  the time it has run (`People::set_clock`, from `Game::advance`). That
  time runs on while the lot is paused or a dialog waits; whether the
  original's clock does is **unverified**. While a bob is on the lot,
  the window draws a new frame every 50 ms even when no tick runs (our
  choice; the original's frame rate is not ported).
- **Colour.** The suit's name sets a mode (`head-arrow` 1,
  `head-arrow-date` 2, `head-star` 3, 0x5856db to 0x58579f). In mode 1,
  m is the mood (motive 3 cut to an integer) × 0.012, held to −1..1.
  - From 0 up, r = b = 1 − m and g = 1, with b lowered a further
    0.4 − m below 0.4.
  - Below 0, r = 1 and g = b = 1 − m², with b lowered a further m + 0.4
    above −0.4.
  - So the bob runs red, pale yellow, green.
- **Light.** The colour is a light of intensity 1, and the ambient light
  is a quarter of the colour plus 0.05. The light's vector is the house
  camera's unit vector from its eye to the centre (+0x184, set at
  0x49b2aa), turned 80° about y and then 30° about x (0x6f6b38). That
  turn is in the lot's terms, so the light falls from another side at
  each rotation of the view.

**Taken from the screenshot, not read.** In the original's screenshot
(medium zoom, 7:38 AM, Mortimer reading on the porch bench), the bob is
brightest on the viewer's left and lit on its lower half. That is what
we get by taking the vector as pointing towards the light. Taken the
other way, the left side is dark. With that choice our bob's green
pixels span the original's box (x 427 to 434, y 74 to 97), each edge
within a pixel, as a gated test checks; the pixels inside the box are
not compared one by one.

**Unverified.**
- Which bone the slot names (its +0x20); `HEAD` puts the bob where the
  screenshot has it.
- Rotations other than 0.
- ~~The bob's pixels name the person who holds it, for a click.~~
  Settled on 2026-11-18: they name the bob
  (`20261118-plumb-bob-picks-as-itself.md`).

**Not ported.** Modes 2 and 3, the date and star bobs.
