# The sun does not turn with the camera

*Decided 2026-11-14.*

The grass at far zoom, rotation 3, matched the original only 12.7%.
The reason was the sun. We tilted it by the camera's rotation (1, 2, 3
and 2 tenths of π from the zenith at rotations 0 to 3), as the setter
at 0x4a4551 does. But only a command handler calls that setter (message
0xdd at 0x4c41bf), and turning the camera does not send that message.
The sun keeps the angle the grass's constructor gives it: π/10 (the
float 0x3ea0d973 at 0x49c187), at every rotation. It still stands
toward the view's +x, since the slopes it lights are read in view
order.

Everything else in the shading was checked against the original's code
and already matched:
- view tiles turn to lot tiles through a 64 by 64 table built around 63
  (0x407704), as `unrotate_tile` does;
- corner heights are reordered per rotation (0x6c4ebc);
- the vertex loop and the blade map count from the lot's least view
  corner, which turning the camera sets (0x4791a6) and the shading
  (0x4a4783, 0x4a48eb) and the blades (0x49ce80) use.

**Result** at far zoom, rotation 3, walls up (the private screenshot
from [20261114-two-more-cameras-compared-original](20261114-two-more-cameras-compared-original.md)). Exact matches:

| Area | Before | After |
|---|---|---|
| Open grass (x 560 to 760, y 440 to 540) | 12.7% | 99.4% |
| House box (x 230 to 480, y 200 to 400) | 55.2% | 74.0% |
| Whole screen | 22.6% | 80.8% |

Fitting the angle alone gives the same answer: 0.09π and 0.11π reach
about 63% to 65%, 0.125π 28%, and 0.1π 99.4%. Rotation 0 is unchanged:
84.4% of the whole screen with walls up. Medium zoom is unchanged too:
76.4%.

**Unverified:** rotations 1 and 2 have not been shot, but the same
constructor value applies to them. Sending message 0xdd (probably a
debug command) would tilt the sun; it is not modelled.
