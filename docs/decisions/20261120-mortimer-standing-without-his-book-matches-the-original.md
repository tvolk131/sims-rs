# Mortimer standing without his book, compared with the original

*Decided 2026-11-20.*

Compared with the live original at 8:05 AM: near zoom, rotation 2,
walls down, scroll (280, −2004), which is camera origin (279, −2004)
here. Mortimer (person 21) has put his book back and stands by the
stairs. Nothing is ported. The test
`mortimer_stands_without_his_book_as_the_original_draws_him` (sims-people,
needs `SIMS_INSTALL`) pins the comparison.

**The animator fields compared.** The original's animator for person
21 was read from its memory with the game paused. These fields equal
ours at tick 2854:

- **Foreground layer** (+0xc):
  - phase 0.5004169 (+0x28);
  - clip time 599 ms (+0x30);
  - duration 1199 ms (+0x20);
  - speed 1 (+0x24);
  - weight 1 (+0x2c).

  Ours plays `a2o-idle-neutral-trans-stand-Mstand` with the same values.
- **Base layer** (+0x14): phase 0.4792362, clip time 957 ms, duration
  1999 ms, speed 1 and weight 1. Ours plays `a2o-standing-loop` with the
  same values.
- **Carry layer** (+0x13c): there is none. Ours has no accessory.
- **Head** (+0x210 to +0x218): heading 0.5969, target heading 5.896 and
  turn 0. Both headings were read to four digits. Person data word 42 is
  8, so the head is not following anything. Ours holds the same values
  to those digits.

Not compared:

- the layers' end modes and other fields;
- the clip names on the original's side;
- the person's position and model matrix;
- the bones the skin computed.

So a pose difference is not ruled out, only made unlikely.

**What matches.** Inside his outline (pixels that are his, with all eight
neighbours his, in both pictures), 544 of 572 are within 24 of the
original in every channel and 435 within 12. No book is drawn in either.

The test checks five interior pixels: the face twice, the hair, the
trousers and a shoe. Each must be within 12 per channel, and the canvas
must name person 21 there. A control frame has the same lot and held
objects without the people pass, and it must not already match.

**What does not: the outline.** Ours sits about half a pixel left of and
above the original's:

- along his legs, our left edge is one pixel further left;
- along his face, the original reaches one pixel further right;
- one row of the original's shoe is one pixel lower.

Ours was redrawn with every person shifted by 0, 0.25, 0.5 or 0.75 of a
pixel right and down; shifts left or up were not tried. The table counts
pixels in his box whose person-or-lot status differs from the original's:

| shift (x, y) | outline | colour off by more than 24 |
| --- | --- | --- |
| (0, 0), as drawn | 148 | 261 |
| (+0.5, +0.5) | 9 | 26 |

Every other shift tested does worse. With the shift, his inside agrees
too: 628 of 629 pixels within 24 and 577 within 12. So the texture moves
with the outline. Bella, on the sofa in the same frame, also fits best
at (+0.5, +0.5): 20 pixels, against 80 or more for every other shift
tested.

The 7:12 AM frame of the bench decision differs in scroll ((592, −2018),
origin (591, −2018)), and also in tick, pose and what hides Mortimer. The
same fit there gives (0, 0) as the best of the shifts tested: 72 pixels,
against 146 or more. (0, 0) is at the edge of that grid. These best
shifts describe those two frames; they do not show what causes the
difference between them.

**What these frames rule out:**

- **A fractional scroll.** The scroll words 0x6f00d0 and 0x6f00d4 are
  integers, and the call at 0x4a7b51 passes them to the camera setup
  0x49b0de. That setup moves the camera along its screen axes by the
  negated scroll (0x49b3cf to 0x49b43d). Then it moves it by half the
  view rectangle's width and height (halved at 0x49b42c and 0x49b462).
  The rectangle was 0, 0, 800, 600.
- **Each Sim's own placement.** In the 8:05 frame two Sims share the best
  shift.

**Open:** why the original's people land half a pixel off ours in one
frame and not in the other. The test that would settle it: hold the tick
fixed, then fit signed shifts at two scroll positions. That run ended
when the game crashed. Where the original turns a person's mesh into
pixels is not read. Its 3D library (0x5c0000 to 0x5e0000) has no `fistp`.

Reopen this when a frame's best-fit whole-figure shift is a whole pixel
or more, or when the original's rasterizer is read.
