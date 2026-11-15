# Objects cast the original's soft shadows on the floor under them

*Decided 2026-11-15.*

At near zoom the most visible difference left was the shadow under
plants, benches and other furniture, which the original darkens into the
floor or grass after drawing each tile and before any object (its floor
shadow pass, 0x4ae844 in the tile draw 0x4ad8ac). It is now ported in
`crates/sims-render/src/shadows.rs`, from the code:

- **One object a tile.** The first object of the tile's object grid cell
  (lot arrays 3 and 103), or the next on the tile through slot 0, that is
  not hidden, has no zero-extent flag (data word 8, value 4), is not of
  type 2 or 9 and has a shadow (0x434e23).
- **The shadow code** (0x4573e7). A one-tile object's is its OBJD word
  57, -1 for none. A tile of a group builds one from which of its eight
  neighbours in the group's grid (OBJD word 11) also have a shadow and
  have OBJD word 56 at 0 (0x43ac5c); a code the masks cannot draw
  becomes 0. It is turned by the object's facing and then the view's
  rotation (the tables 0x53d2fc fills); code 16 never turns.
- **The masks.** Codes 0 and 16 are hand-drawn, SPR2 0x300 and 0x310
  (plus 0x20 a zoom) in `floors.iff`, read as palette indices. Every
  other code's mask is worked out over floor 0's tile shape from four
  curves, one a side, as the original's float code does it (0x4a3cd3,
  0x4a3914, 0x4a3993).
- **Drawing** (0x4b10c4). Mask byte plus strength (OBJD word 79, plus
  255 less the tile's drawn light byte) gives a light level for the
  pixel, which is drawn through the lighting's tables; 248 and up leaves
  it alone. Only pixels at or behind the floor's ground depth change.

**Checked** against the original's screenshots, with the world at their
time. Near zoom rose from 86.3% to 87.6% exact (two other near cameras
86.8% to 88.3% and 86.5% to 87.1%); far zoom at rotations 1 and 2 by
0.1 to 0.2 points and medium zoom from 82.7% to 83.1%. Of the pixels the
shadows changed, 6,359 of 7,149 now match exactly at near zoom (950 of
1,186 at far rotation 1, 449 of 574 at rotation 2, 2,046 of 2,755 at
medium); before, almost none did. The bench's two halves meet in one
shadow, which checks the turning's sense.

**Not ported:** the option that turns shadows off (the "shadows" cheat)
and the drawing flags that allow them, taken as on; the faint shadow
under an object the player is placing (0x438a48). The pass runs in the
tile order of our floor loop, after all grass, where the original runs it
tile by tile in its own order, so a later tile's grass blade can cover a
shadow there and not here.

**Seen, not yet read:** on a split tile whose other half has no floor,
the original draws that half as flat grass (two tones, no blades); we
draw the bladed grass. The dashed row along the porch's lower edge at
near zoom is ours alone.
