# A person upstairs takes the view's level offset

*Decided 2026-11-15.*

A person on a level above the ground is now moved by the original's own
level offset (`sims_people::lot::LEVEL_OFFSET`, `View::place_on`), not by
the lot renderer's floor height alone. The original's model matrix
(0x592132, at 0x5922d2) adds (level − 1) times a vector from the house
view: the three floats at the view's +0x170 − 12·zoom (its virtual +0x24
returns +0xb8; +0x14 is the zoom). The table is filled at run time, so it
was read from the running original's memory (the Wine process, through
the global at 0x6e48bc): (0.5, 9.300511, 0.5) model units at all three
zooms, the same at zoom 1 rotation 0, zoom 2 rotation 0 and zoom 3
rotation 1.

The original turns the vector by −rotation quarter turns about the
vertical (0x5c3a20, axis 1) before adding it. The view's own matrix at
+0x130 is built from the same turn, ((−rotation) & 3) quarter turns
(0x49b60e), and that matrix turns the view's frame into the lot's (the
standard light keeps its place relative to the viewer through it). So
the offset is fixed in the view: 9.300511 units up, and half a unit,
a sixth of a tile, towards the viewer along both of the view's axes,
at every rotation.

On screen that is 243.00 near-zoom pixels up, less 10.67 for the sixth
of a tile on each axis: 232.33 per level, a third of a near pixel more
than the floor height the renderer lifts a level by (58.08, 116.17 and
232.33 at far, medium and near). It rarely moves a pixel. The real
change is depth: a person upstairs now stands a sixth of a tile nearer
on both axes, and 11 near pixels higher, than the floor they are on.
We keep the renderer's level in the depth and the projection, and add
only what the original's offset has beyond the floor height, because
`Depth::point` holds an elevation to the terrain's curve.

Checked against the 7:29 AM screenshot that
`20261115-person-upstairs-stands-terrain-too.md` used (medium zoom,
rotation 0, level 2): Bella at the dollhouse scores the same as before
(1,223 of 2,250 pixels exact in her box, against 1,226). The sign is
pinned by that screenshot: the other sign would put her 126.8 pixels up,
11 too high. The rotations are read from the code, not compared with a
screenshot of a person upstairs at rotations 1 to 3.

This replaces what `20261115-person-upstairs-stands-terrain-too.md` said
of the view-table offset: that it is not ported.
