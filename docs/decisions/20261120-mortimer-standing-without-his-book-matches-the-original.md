# Mortimer standing without his book matches the original, but for a half-pixel outline

*Decided 2026-11-20.*

Compared with the live original at 8:05 AM: near zoom, rotation 2,
walls down, scroll (280, −2004), which is camera origin (279, −2004)
here. Mortimer (person 21) has put his book back and stands by the
stairs. Nothing is ported. The test
`mortimer_stands_without_his_book_as_the_original_draws_him` (sims-people,
needs `SIMS_INSTALL`) pins the comparison.

**Same moment.** The original's animator for person 21, read from its
memory with the game paused, equals ours at tick 2854:

- foreground layer (+0xc): phase 0.5004169 (+0x28), clip time 599 ms
  (+0x30), 1199 ms long. Ours plays
  `a2o-idle-neutral-trans-stand-Mstand` at the same phase and time.
- base layer (+0x14): phase 0.4792362, clip time 957 ms, 1999 ms long.
  Ours plays `a2o-standing-loop` at the same phase and time.
- no carry layer (+0x13c), and no accessory in ours.
- head (+0x210 to +0x218): heading 0.5969, target heading 5.896, turn 0;
  person data word 42 is 8, so the head is not following anything.
  Ours holds the same values.

So any difference in the picture comes from the drawing, not the pose.

**What matches.** Inside his outline (pixels that are his, with all eight
neighbours his, in both pictures), 544 of 572 are within 24 of the
original in every channel and 435 within 12. No book is drawn in either. The test checks
five interior pixels (the face twice, the hair, the trousers, a shoe),
each within 12 per channel. It also checks that the lot alone does not
already match there.

**What does not: the outline.** Ours sits about half a pixel left of and
above the original's. Along his legs our left edge is one pixel further
left; along his face the original reaches one pixel further right; one
row of the original's shoe is one pixel lower.

Ours was redrawn with every person moved by a fraction of a pixel. Pixels
whose person-or-lot status differs from the original's, in his box:

| shift (x, y) | outline | colour off by more than 24 |
| --- | --- | --- |
| (0, 0), as drawn | 148 | 261 |
| (+0.5, +0.5) | 9 | 26 |

Every other quarter-pixel shift from 0 to 1 does worse. Shifted, his
inside agrees too: 628 of 629 pixels within 24 and 577 within 12. So the
texture moves with the outline. Bella, on the
sofa in the same frame, also fits best at (+0.5, +0.5): 20 pixels,
against 80 or more for every other shift.

At the 7:12 AM camera (scroll (592, −2018), origin (591, −2018); the
frame of the bench decision), the same fit gives (0, 0) as the best, at
72 against 146 or more. So the half pixel is not a fixed difference in
how pixels are covered.

**Ruled out:**

- The time and the pose. The layers and the head are the original's to
  seven digits.
- The head turn. It is 0 in both.
- A fractional scroll. The scroll words 0x6f00d0 and 0x6f00d4 are
  integers. The camera setup (0x49b0de, called from 0x4a7b51) puts the
  camera at the screen's centre less the scroll (0x49b3cf to 0x49b4aa). It
  takes the centre as half the view's rectangle (0x49b103), and the
  rectangle was 0, 0, 800, 600.
- The person. Two people in the same frame shift alike.

**Open:** what makes the original's people land half a pixel off ours at
one camera and not at another. A second scroll at the same tick would
settle whether the camera causes it. That run ended when the game
crashed. Where the original turns a person's mesh into pixels is not
read; its 3D library (0x5c0000 to 0x5e0000) has no `fistp`.

Reopen this when a frame shows a person's outline off by a whole pixel
or more, or when the original's rasterizer is read.
