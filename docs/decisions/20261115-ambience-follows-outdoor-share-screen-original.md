# The ambience follows the outdoor share of the screen, as the original works it out

*Decided 2026-11-15.*

The camera value that moves the ambient score's row is ported from the
original's code (0x570c3e, which its sound manager runs each frame at
0x574161): `sims_audio::ambience::view`. It walks the screen in rows half
a tile high (`height / (4 << zoom)` rows of `width / (16 << zoom)` tiles),
from the view tile under the top left pixel, and counts the tiles on the
floor shown whose tile flag 4 is set, "outside every enclosure" (read by
0x489260, which first turns the view tile back by the camera's rotation).
The value is `outside / counted * 100.0` on the FPU, cut toward zero; with
nothing counted the division gives NaN, which `__ftol` makes 0. Below 30
the player takes it as 0, as before.

`sims_sim::World::outside(level, tile)` is the read: the tile's flag 4
in arrays 8 and 108, as the last room rebuild left them. It only reads,
so the world's hash and strict mode's output are unchanged. `sims-game`
sends `Game::ambience_view` to the mixer with the hour and speed, in
place of the fixed 50 it used. On the Goth house at 800 by 600 it gives
about 92 at the far zoom, 79 at the medium zoom and 26 to 37 near.

**Unverified.** Which tiles count: the walk takes a view column and row
from one past the lot's bounds (lot +0x70 to +0x7c) to one short. The
bounds are taken as the 64-tile arrays' 0 and 63 (tiles 1 to 62), which
is the same in every rotation; `sims_vm`'s on-lot check makes the same
compare with the same unverified values. The corner tile is asked of the
original's camera (its function 4, with 1) and taken here as the ground
level's fine position under the pixel; and the screen size is the house
view's. None of the values has been compared with the running game.
Reopen when a run with sound can be compared, or when the lot's bounds
are read.
