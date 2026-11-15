# The cutaway cuts the rooms the original is looking at

*Decided 2026-11-14.*

Until now the cutaway cut every wall with a room on the camera's side.
The original cuts far fewer, and the rule is now read from its code (the
memory capture, as for the grass), following the old prototype's ports
in `legacy/crates/sims-sim/src/lot_presenter/`.

- **Which rooms.** The view keeps a list of three rooms, newest first,
  that never holds a room twice (0x49e923; its size is set at 0x4a6d96).
  Each pointer move in the cutaway adds the room under the pointer, then
  the first other room straight below it on screen, looked for every
  half tile down to half a storey (0x4ab7ee). The pointer is placed on
  the terrain as drawn, by the original's tile search (0x4a955d, 0x4a97c3).
  Off the lot, or on its outer ring of tiles, the mask is emptied
  (0x4ac07b) but the list is kept for the pointer's return. A change of
  floor empties the list (0x4a80c7). In live mode the selected Sim's room
  is added too, when the Sim is on the floor shown (0x4ab992). That is the
  room the simulation keeps for the Sim, which on a split tile is not the
  tile's. So the choice depends on the pointer and on who is selected: it
  is input, not lot state. `Camera::focus` carries it, and `Focus::point`
  does the pointer's part.
- **The mask.** Those rooms become a mask of view tiles (0x468203). Each
  of a room's tiles is in it, and so is every tile below one of them on
  screen whose standing walls would overlap it. An outdoor room adds no
  tiles of its own. An outdoor selected Sim adds a square of up to 4 by
  4 tiles towards the camera. If any of the rooms is outdoors, the
  pointer adds a 4 by 4 square: from the tile four tiles' height below it
  on screen, four tiles back up.
- **Which walls.** A wall is cut where the mask holds the tiles on both
  sides of it (0x54059a; diagonals have their own offsets). Fences and
  the like (styles 2, 12, 13, 14, 255) are never cut. Only the top floor
  shown is cut; the floors below stand (0x4a8895). Walls-down now follows
  the same floor rule.
- **Objects.** An object that hides with the walls hides when the mask
  holds its tile and the tile its facing side looks onto (0x4aa446). The
  mark is flag 0x400 of data word 8. Doors and windows set it in their
  own scripts as the lot loads, so a house read without a load lacks it.
  There, objects that cut their shape into a wall still hide, as before.
- **Cut doors.** A cut wall of style 3, 5, 6, 15, 22 or 23 draws a plain
  wall's stub. The original marks those styles with bit 8 when it cuts
  them. Doors stand in style 3 walls, whose own cut sprite is an outline.

**Result at the compared camera** (far zoom, rotation 0, floor 1, the
settled cutaway screenshot). Which rooms the original had in its list
cannot be read from the screenshot. The best fit is the kitchen (room 1)
from the pointer, plus the bedroom, where the selected Sim stands.
Adding the living room or the bathroom lowers the match. The foyer and
the bathroom now stand, as in the original. Exact matches:

- Box around the house (x 120 to 500, y 100 to 320): 61.0% before.
  65.6% now, with the house read as before; 67.1% with the world after
  one tick of `sims boot`, whose doors and windows carry their flag.
- Whole screen: 81.2% before, 82.0% now, and 82.4% after a boot.

**What still differs:** the trim along the top of cut walls (since
added; see the entry above), the Sims,
and the light on a few walls. The bedroom's west wall, with its door,
is cut in the original but stands here. The hall behind it, under the
stairs, is outdoors in our rooms, and no mask from an indoor room
reaches west of a room. So either the original counts that hall as a
room, or its list held something else.

**Unverified:**

- Rotations, other zooms and the upper floor. The tests check that a
  looked-at room loses exactly its front walls at every rotation.
- Diagonal walls. The offsets follow the old port, and the house has
  none in a cut room.
- The pointer's 4 by 4 square, and the terrain-aware pointer, against
  the original: this house's lot is flat where it was compared.
- Whether the list also empties on a rotation or a zoom. Other callers
  of the clear (0x4a81a1, 0x4a8517, 0x4a85fa, 0x4a86c2) were not read.
