# The grass is drawn as the original draws it

*Decided 2026-11-14.*

The speckled grass is gone. The ground is now drawn the way the original
draws it, read from its code. The installed game is encrypted, so the code
was read from a memory capture of the running game in the harness's
private cache; nothing from it is in the repository. The addresses below
are in that image.

- **Palette** (0x49c8d3, 0x49e6cf). 32 grass colors run from brown to
  green. Each has 128 shades from light to dark. The light end blends the
  light brown 0x9d7541 into the light green 0x50743b; the dark end blends
  0x513c12 into 0x0e3508. A shade is a 4 by 2 block: two of its pixels
  are 8 and 6 shades lighter, and the second row is mirrored. Channels
  are truncated, then cut to 16-bit color.
- **Shade at each vertex** (0x4a4994, 0x4a4783, 0x4c47f7). The light on
  each of the six triangles around a vertex is the squared cosine between
  the sun and the slope. A tile is 2√6 height units wide (0x434df6). The
  sun stands π/10 from the zenith at rotation 0. Rotations 1 to 3 use 2,
  3 and 2 tenths. The mean becomes a shade, 127 − ⌊127·mean⌋. A jitter of
  up to two shades is added from the game's generator, seeded 0x40de9. A
  wall on the tile's far edge darkens that edge by 64 shades.
- **Grass color** (0x49cd43). Each corner averages the grass values of
  its four tiles and scales them to 0 to 31.
- **Columns** (0x49dacd, 0x49da21). A tile is drawn in four-pixel
  columns. Shade and color are interpolated down each column in 256ths.
- **Blades** (0x49dfbd, 0x49d79d, 0x49d8fb, 0x49cf9f). 4800 blades are
  scattered over a tile's shape from seed 0x4aa47b44. Each has a rank, a
  column, a depth down it, a shade 0 to 60 darker than the ground and a
  height (one pixel at the two far zooms, three or four near). A tile
  draws the blades whose ranks fall in a window. The window's size
  follows the tile's grass value: 500 to 1200 blades, a quarter of that
  at far zoom, none on a floor. Its start comes from a random map seeded
  0x4aa47b44.
- **The lot's size matters.** The blade map and the jitter loop both run
  over the lot's own size, not the arrays' 64 (0x49c9c7, 0x4a4a43). The
  Goth house is 52 tiles. That size sets which random draw each tile and
  vertex gets. `open_house` now applies the size the house asks for in
  global 23, as the load's resize does (`Interpreter::resize`). Before
  this, fitting single tiles found the blade counts right and the starts
  wrong. The starts matched the map at 52 entries a row. Fixing the
  jitter loop took the grass box from 84% to 100%.
- **Edges.** Ground is drawn only from tile 1 to the lot's size less 2
  (0x49cecc). Beyond the lot the window shows the original's grey, 16-bit
  (14, 28, 14), which is now `BACKGROUND`.

**Result at the compared camera** (far zoom, rotation 0, floor 1, 800x600,
the screenshots of the lighting entry below). Exact matches:

- In a box of open grass (x 530 to 640, y 335 to 365): 0% to 100%,
  in both wall modes.
- The whole screen: 18.1% to 83.6% with walls up, and 16.1% to 81.2%
  with walls cut away.
- The box around the house (x 120 to 500, y 100 to 320): 72.2% with
  walls up and 61.0% cut away. It was 37.5% and 26.3%.

Nearly every open grass tile now matches at least 90%. **What still
differs:** the house and its objects (lamp pools, the cut-wall cap,
some gravestone sprites), Sims, which are not drawn, and the game's
control panel in the bottom-left corner.

**Unverified:**

- Other zooms (taller blades near) and rotations. In particular, whether
  the blade map and the jitter loop count from the lot's least corner in
  the view, as here; the pool test at 0x47c16d suggests view
  coordinates. At rotation 0 that corner is 0 either way.
- Depth on sloped ground. A tile's ground takes the depth of its lowest
  corner. Following the slope would hide sprites behind hills better, but
  floors take their top corner's elevation, and the grass then covered a
  floor's edge where sloped ground meets it (0.1% fewer exact pixels).
  One depth model for floors and ground would settle it.
- Grass on a pool, which the original leaves bare. Pools are not looked
  at.
- A floor tile split by a diagonal wall. Now ported; see "A split floor
  tile's grass keeps the blades of the tile drawn before it"
  (2026-11-15).
- Seasons, which swap the blade-height tables (0x6c629c, 0x6c62a8).
- A flag at 0x8d that blanks the colors of tiles on the lot's edge. It
  is off in this capture and not modelled.
