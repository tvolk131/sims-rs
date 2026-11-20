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

**How pixels were counted.** The original's screenshot does not say
which pixels are a person's, so for the counts below both pictures use
the same estimate: a pixel counts as a person's when it differs from
our people-free render of the same tick (the lot and held objects, no
people pass) by more than a cutoff in some channel. A person's pixel coloured like the lot behind it counts as lot.
The original's screenshot is 16-bit colour.

- *Interior* (cutoff 40): pixels in his box (x 415 to 479, y 225 to
  391) that are his, with all eight neighbours his, in both pictures.
- *Outline* (cutoff 30): pixels in a box whose person-or-lot status
  differs between the original and ours.
- *Colour*: pixels in the box more than 24 apart in some channel.

**What matches.** Of his interior pixels, 544 of 572 are within 24 of
the original in every channel and 435 within 12. No book is drawn in
either. The same `People` follows the lot every tick, so his rendered
body loads `suit-book101` while he carries it and drops it by 8:05.

The test checks five interior pixels: the face twice, the hair, the
trousers and a shoe. Each must be within 12 per channel, and the canvas
must name person 21 there. A control frame has the same lot and held
objects without the people pass, and it must not already match. It also
checks eight points on his outline where both pictures put the edge on
the same pixel (the inside pixel his, its outside neighbour not). Moving
his pixels one pixel up, down, left or right fails at least one of them.

**What does not: the outline.** Ours sits about half a pixel left of and
above the original's:

- along his legs, our left edge is one pixel further left;
- along his face, the original reaches one pixel further right;
- one row of the original's shoe is one pixel lower.

**The grid.** Ours was redrawn with every person moved by 0, 0.25, 0.5
or 0.75 of a pixel right and down (16 renders). In his box:

| shift (x, y) | outline | colour |
| --- | --- | --- |
| (0, 0), as drawn | 148 | 261 |
| (+0.5, +0.5) | 9 | 26 |
| next best, (+0.5, +0.25) | 37 | 94 |

With the shift, his interior agrees too: 628 of 629 pixels within 24 and
577 within 12. So the texture moves with the outline. Bella, on the sofa
in the same frame (box x 600 to 699, y 30 to 199), also fits best at
(+0.5, +0.5): an outline count of 20, against 80 or more for every other
shift in the grid.

Separately, some exploratory renders moved the people a quarter pixel up
(y −0.25) or a whole pixel right (x 1). None beat (+0.5, +0.5) for either
Sim. Shifts further left or up were not tried.

**The 7:12 AM frame** of the bench decision: scroll (592, −2018), origin
(591, −2018), our tick 1260, Mortimer seated. The same grid, scored with
the outline rule in the box x 360 to 430, y 250 to 330 around him,
against the original's screenshot, gives (+0.5, +0.5) as best: 14
pixels, against 94 at (0, 0) and 21 or more for every other shift.

An earlier version of this decision said this frame fit best at (0, 0),
72 pixels against 146. That was scored against a file that is one of our
own renders, saved by mistake under the original's name (98 pixels off
our tick-1260 frame, about 43,800 off the screenshot). So it compared
ours with ours. Scored against the screenshot, the whole-frame mask that
gave those numbers counts about 1,400 for every shift: it takes in other
differences in that frame and does not separate the shifts.

So two frames, at different scrolls, ticks and poses, both fit best with
our people moved half a pixel right and down. That is consistent with a
fixed offset in how one of the two programs places or rasterizes a
person, but these two frames do not show that it is fixed.

**What these frames rule out:** a fractional scroll. The scroll words
0x6f00d0 and 0x6f00d4 are integers, and the call at 0x4a7b51 passes them
to the camera setup 0x49b0de. That setup moves the camera along its
screen axes by the negated scroll (0x49b3cf to 0x49b43d). Then it moves
it by half the view rectangle's width and height (halved at 0x49b42c and
0x49b462). The rectangle was 0, 0, 800, 600.

**Open:** why the original's people land half a pixel right of and below
ours. Two Sims in one frame sharing the best shift does not show whether
placement or rasterizing causes it. Where the original turns a person's
mesh into pixels is not read. Its 3D library (0x5c0000 to 0x5e0000) has
no `fistp`. Next steps: fit signed shifts in more frames, including two
scrolls at one tick (that run ended when the game crashed), or read the
original's rasterizer.

Reopen this when a frame's best-fit whole-figure shift differs from
(+0.5, +0.5) by half a pixel or more, or when the original's rasterizer
is read.
