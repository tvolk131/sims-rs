# Rotations 1 and 2 and near zoom, compared with the original

*Decided 2026-11-14.*

New screenshots of the Goth house in the original: far zoom at rotations
1 and 2, and three near-zoom views at rotation 0 (the front wall and
door, the porch, the bay window), all with walls up. The camera origins
were found by search; each places the house to the pixel. House box,
exact matches in 565 space:

| Camera | Exact |
|---|---|
| Far zoom, rotation 1 | 85.5% |
| Far zoom, rotation 2 | 86.1% |
| Near zoom, front wall and door | 90.4% |
| Near zoom, porch | 80.5% |
| Near zoom, bay window | 56.2% |

The geometry lines up at every one: walls, openings, trim, floors,
objects and grass. Most of what is left is not drawing:

- The shots were taken at 6:49 in the morning, later than the earlier
  ones. In the original the stair room (the one with the bay window)
  is drawn at about light-table level 20 (see `Light`), judged from its
  pixels; ours uses level 16, the half-light floor. Its walls, floor
  and stairs make up nearly all the misses at rotations 1 and 2 and in
  the bay view. The room's light is the simulation's, so the render
  leaves it.
- The Sims in the shots, which this crate does not draw.

One drawing difference remains in these new views (the door jamb's
depth, recorded below, is separate): a diagonal wall seen edge-on (frame 3,
the bay's left end) shows other texels than ours over its width, 4
pixels at far zoom and 16 at near. Showing the wall's other side did
not match either, so it is not only the side. The original picks the
side from the run's direction (0x53fd47), and for an edge-on line looks
up its own side's wallpaper by a fixed view direction (0x53f8ce); the
texel offset was not read. Reopen with that offset.
