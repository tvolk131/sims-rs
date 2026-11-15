# People's meshes go into the lot renderer's canvas

*Decided 2026-11-15.*

`People::draw_on_lot` draws a lot's people into the canvas
`sims_render::draw` has filled, as the render track found the original
does: its depth buffer, dumped with Sims on screen, holds only the lot.
Each mesh vertex takes the lot renderer's `Depth::point` for its fine
view position, elevation and level, interpolated across the triangle,
and each pixel goes in by `Canvas::put`'s sprite rule (no farther than
what is there, ties to the person; fainter than 65 left out; soft pixels
blend; depth written so a nearer part of the same body covers a farther
one). Depths are cut toward zero, as the original's conversion does and
as `Depth::anchor` does. The screen position is the renderer's
`Camera::project_fine_f32`, one column right.

Checked in the booted Goths' lot, each person drawn alone (ticks 120 to
480, four rotations, medium zoom, the camera on the ground level, walls
down): their feet show in 44 of the 53 views where they are on screen.
In the three views looked at where a person or their feet are hidden,
a hedge with the porch railing, a bookcase or the bed's posts stands in
front; the others were not looked at. Feet show because a floor lies 65 units farther than the
point under it, and grass, held at its tile's top corner, 80 or more
(200 to 650 at five grass pixels probed). With the camera on level 2
the upper floor hides everyone below it, as it hides the rooms. How the
original gives a person's mesh its depth is still **unverified**; reopen
when a person behind furniture is compared with the original.
