# A lit lamp throws a pool of light on the floor around it

*Decided 2026-11-15.*

The bay's floor was lit in graded pools the renderer did not draw. They
come from the lamps, not the windows: in the original a lit lamp marks
the tiles around it, and each floor tile it reaches adds a pool map from
`GameData/lightMap.dat` to its light. `sims_render::pools` reads the
maps, `Lot::lamps` keeps each tile's lamps, and the floor draw adds the
pools. A lit lamp's own sprite is drawn in full light.

**Read from the code.**

- The marks (0x467e05, 0x4b4e8b, 0x4b5037). As the room update counts
  an object, one that is a light source (data word 40 flag 0x400), gives
  light (word 51 not 0) and has word 16 set to 1 takes its marks off and
  marks again: every tile within 4 tiles each way (12 / 3, rounded), on
  the lot's inner tiles and in the lamp tile's room (for a split tile,
  either half's), takes the offset from the lamp. A tile keeps two: a
  third lamp nearer than the first replaces the first (which is lost,
  not moved down), else one nearer than the second replaces it.
- The pools (0x4c811f, 0x4c8049, 0x4b6c17, 0x4b61b2). A floor tile whose
  room light is not 1 (after the 0.5 to 1 clamp) draws with each lamp
  within 3 tiles each way (the map's reach, 0x4a3805), the view-rotated
  offset choosing where in the map the tile's image lies (0x4a3813). A
  pixel's light byte is `fraction * 255 + 0.5`, truncated, plus each
  lamp's map byte, capped at 255; its top five bits pick the level as
  usual, so the brightest pool is level 31, never full light.
- The file (0x4b5700). Four light fields, then the wall tables, then a
  pool map per zoom. The light radius of 12 gives the field and table
  sizes (0x49bdf6, 0x49be1f, 0x4bb7c5), and the parts then add up to the
  file's 2,271,104 bytes exactly. That is how the maps' sizes were
  found: 7 tiles across and half that high at each zoom.
- A light source other than a window (type 8) with light (word 51) draws
  in full light (0x437f7d, 0x4b455f).

**Compared** with the world at the screenshots' time (a snapshot at tick
570, 6:49): near zoom rose from 85.2% to 86.1% exact (non-green 90.9% to
92.0%), and far zoom at rotation 2 from 89.9% to 90.1%. Far zoom at
rotation 1 and medium zoom do not change: no pool shows in them. The
floors in the bay now match; what differs there is the edge-on wall
(fixed in #159) and the shrub's shadow.

**Not verified.** The original builds its marks up as it counts rooms,
in its own order, and only takes a lamp's marks off when it next counts
it dark; we work them out afresh, lamps in object order. They could
differ where three or more lamps reach a tile or a lamp went dark; no
screenshot shows either. Which way the record turns with the view
(0x4aedc2) is not read: we turn each offset as the camera turns the
lot, so a pool stays centred on its lamp. Turned the other way, a tile
off the lamp's axes would show its gradient reversed; rotation 2 cannot
tell the two apart, and no screenshot at rotation 1 or 3 shows a pool. When
the file is missing the original computes the maps (0x4bb5bc); we draw
no pools. Data word 16's meaning is not known.

**Not ported.** Pools on walls (0x4c78c5: the file's wall tables, by
the lamp's distance from the wall and its offset along it, blitted by
0x4b7630 and 0x4b780d), on grass, and the full light a draw group's
sprite gets when a word of its record is 7 (0x437adb).
