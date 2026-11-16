# Rooms darken by their light in the lot drawing

*Decided 2026-11-14.*

Floors, walls, terrain and objects are now drawn darker in a dim room, as
the original draws them.

- **Which light.** Each room's shown light comes from the simulation
  (`Room::shown_light`, read only). As in the original (0x46772d), it
  becomes a byte, `light * 255` rounded half up. Drawing reads the byte
  back as a fraction and keeps it between 0.5 and 1; that clamp is the
  old prototype's reading of the drawing context
  (`legacy/crates/sims-vm/.../draw/context/light.rs`). So even an unlit
  room shows at half brightness. Outdoor rooms the light pass never
  counted are drawn at full light.
- **Where it is read.** A floor half takes the light of the room under
  it; a split tile's two triangles each take their own room's. A wall
  takes the room just beside its middle on the side that shows, so a
  split tile gives the triangle touching the wall. An object uses the
  room at its position. Terrain uses floor 0's room at the tile centre.
- **How it darkens a pixel: fitted, not read from code.** The original
  draws in 16-bit color. Dividing its pixels by ours in the Goth house
  gave a sharp peak at 0.517 indoors and 1.0 outdoors. The booted
  snapshot's indoor rooms all have light at most 0.127, so they sit at
  the 0.5 clamp. Fitting each 16-bit channel gave
  `(v * k + 255) >> 8` with k = light in 256ths for red and green and
  one more for blue. That reproduces 69% of the indoor pixels exactly.
  The other 31% are mostly one step off.
- **Result at the compared camera** (far zoom, rotation 0, floor 1,
  800x600). Exact matches among non-grass pixels:
  - walls cut away: 33.9% to 42.2%;
  - walls up: 39.7% to 47.3%.

  In the box around the house (x 120 to 500, y 100 to 320), exact
  matches rose from 6.8% to 26.3% with walls cut away and from 20.5% to
  37.5% with walls up.
- **What still differs.** The grass (ours is speckled; the original's is
  smooth and shaded) and the wood cap on cut-down walls. Some gravestone
  sprites differ, and Sims are not drawn. Some pixels in the house are
  lit between 0.6 and 0.9 of ours. They look like pools of lamp light,
  probably from the falloff tables in `lightMap.dat`; they are not
  modelled.
- **Unverified.** Every light level other than the 0.5 clamp and full
  light. Night outdoors. Whether the original lights a wall by the room
  on its visible side. Lamp pools. Any of these would reopen the formula.
