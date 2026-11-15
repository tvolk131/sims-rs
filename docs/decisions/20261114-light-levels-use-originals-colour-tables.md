# Light levels use the original's colour tables

*Decided 2026-11-14.*

Lit pixels were often one step off from the original: window and door
frames, porch floors, walls in shade. The cause was the formula, not the
light. Read from the original's code:

- A sprite's light is a float from 0 to 1. If it is within 1e-6 of 1,
  the sprite is drawn as is (0x4c7c59). Otherwise the blitter turns it
  into a byte, `L = trunc(light * 255.0 + 0.5)` (0x4b76a8), and picks a
  table by `l = L >> 3` (0x438e6d).
- The tables (built at 0x49eea8) map each 565 channel `v` to:
  red `trunc(l * v / 31 + 0.5)`, green `trunc(2l * v / 63 + 0.5)`, blue
  `trunc((max(l, 1) - 1) * v / 31 + 1.5)`. The divisors are the f32
  constants 1/31 and 1/63.
- So blue is not scaled like red: at half light (`l = 16`) a blue of 0
  becomes 1 and every even blue gains one step, and even at `l = 31`
  black's blue is 1. That is the one step we were missing.

`Light` in `crates/sims-render/src/light.rs` now holds the table index
(or full light) and applies these formulas. House box, exact matches:

| Camera | Before | After |
|---|---|---|
| Medium zoom, rotation 0 | 89.3% | 90.4% |
| Far zoom, rotation 0 | 89.5% | 90.2% |
| Far zoom, rotation 3 | 88.8% | 90.0% |
| Far zoom, cutaway | 85.8% | 87.1% |

At medium zoom, lit pixels now match 93.6% (from 89%); most of the rest
are placement and depth.

**Unverified:** the two shrubs by the bay window (and 16 other objects
in the same room, mostly fences and flowers) are drawn at half light
here and at full light in the original. The simulation puts them in room
4, which it marks indoors with light 32. Either the room's outdoors flag
or the tile light the original keeps for unfloored tiles differs; that
is the simulation's data, so the render does not override it. Reopen
when the simulation's room lights are checked against the original.
