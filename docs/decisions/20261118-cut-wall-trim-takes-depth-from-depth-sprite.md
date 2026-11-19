# A cut wall's trim takes its depth from its depth sprite

*Decided 2026-11-18.*

With walls down, the wood trim on the cut walls of a bedroom drew over
the bed in front of them. The original draws the bed over the trim.
Ours gave every trim one depth a row, taken from the top of a standing
wall. A cut wall is a few pixels tall, so its trim was far too near.

## What the original does

The trim pass (0x53db67) draws a standing wall's trim (0x4c8849, SPR#
4096 + zoom) at one depth a row, as we did. A cut wall's trim (branch
at 0x53e2fa) goes through 0x4c8b1f instead. It draws the colour sprite
SPR# 5120 + zoom together with a depth sprite, SPR# 6144 + zoom, frame
for frame. The two have the same bounds in every frame. The blitter
(0x49ecbc) gives each pixel the depth

```text
L[w9 + (8 << zoom) - y] + w10 + layer(byte, column)
```

with `w9` and `w10` the trim's anchor and `L` the upright rows, as for
a wall. The depth sprite's byte picks one of eight layers (table
0x668440). Each layer adds a column table and a step:

| bytes   | column table            | step   |
|---------|-------------------------|--------|
| 0–62    | none                    | T      |
| 63–88   | none                    | T / 2  |
| 89–113  | none                    | T      |
| 114–139 | none                    | T / 2  |
| 140–190 | C from 16 << zoom       | 0      |
| 191–215 | C from 8 << zoom, less 1 | T      |
| 216–254 | C from 8 << zoom        | T      |
| 255     | not drawn               |        |

- `C` is the wall column table: the depth owner's word 14 − zoom.
- The column runs from the sprite's left.
- `T` is the owner's word 18 − zoom: C's peak, `unit + 0.5` truncated
  (0x4ca10a). That is 462 at every zoom.
- A pixel is drawn if its depth is at most what is there plus one, as
  for a wall.

**The anchor** comes from 0x4b9613 (from 0x53e24d). It places the
trim's context like an object, at the middle of the view tile whose top
corner the trim's corner is: fine position (16x + 8, 16y + 8), on the
terrain there, on the corner's level. The old prototype's
`geometry_side.rs` reads the same point. We take the ground there as
the mean of the tile's corners, as we do for objects.

**The light.** Every trim, standing or cut, is lit with one light: the
light shown by room 0, the outdoors (0x466fcc(0), then 0x4688bf), or
full light without one (0x53dbb4 to 0x53dbec). The original sets it at
0x53e2f5, held between half and full. We drew trims in full light. At
night the original's trims are half as bright.

## What we do

- `Depth::trim` is the blitter's depth.
- `Assets::trim` attaches each cut frame's depth bytes.
- `Canvas::draw_layered` draws with them, at a slack of one.
- `trim_anchor` places the anchor.
- Trims take room 0's light.

## Checked

- Against the original's 12:24 AM screenshot: near zoom, rotation 0,
  walls down, Bella asleep in the master bed. The bed, nightstand and
  lamp now draw over the back wall's trim. The trim caps each stub, as
  in the original. The trim is as dark as the original's.
- Aligned to the pixel, the share of pixels within 8 of the original's
  rose from 61.4% to 66.6% over the room. The rest is grass and the
  sleeping Sim.
- One trim pixel over the far room's floor, compared by depth:
  - Anchored at the corner, ours came out at 48390, behind that floor
    (48375).
  - The original draws the trim there.
  - Anchored at the tile's middle, ours comes out at 47927 and wins, as
    in the original.
- Six of the pinned cutaway renders changed, by 23 to 93 pixels each,
  where a cut trim meets something nearer. The walls-up renders did not
  change: in daylight, room 0 is in full light.

## Unverified

- The ground at the anchor. The original looks the terrain up at the
  tile's middle (0x478dfe). The mean of the corners is the same on flat
  ground.
- The depth values against the game's depth buffer. They were checked
  against its screen only.
- The diagonal cut trims drawn from the standing set (bits 0x50) still
  use the flat depth. The original draws its extra pieces for bit 0x10
  through 0x4c8b1f too (0x53c593, 0x53c630). Ours draws those pieces
  from the cut set, so they take the depth sprite. Its main 0x50 piece
  is not traced.
