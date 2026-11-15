# A lit lamp's pool reaches the walls beside it; grass takes none

*Decided 2026-11-15.*

Straight walls now take lamp pools, from the wall tables in
`lightMap.dat`, as the original's wall pass draws them (read from its
code: 0x53db67 calling 0x4c79ca). A wall reads the lamp record of its
tile in the view: the tile whose low-x edge (a wall along view y,
direction 1) or low-y edge (along view x, direction 2) it stands on,
which is the side the camera sees (0x4aed23). Each of the record's two
lamps on that side of the wall (a view offset from the lamp to the tile
of at most 0 across it) gives an offset along the wall and a distance
out from it: for direction 1 `-dy` and `-dx`, for direction 2 `dx` and
`-dy` (0x4c78c5). A lamp counts if the tables hold an image for it: a
distance below 4 and an offset within `m - 1` of the middle, for the
distance's `m` of 5, 5, 4 or 3 (0x4b5c98). The image is direction 1's
table or direction 2's, at the zoom and distance, image `offset + m - 1`
(0x4b5c0c, 0x4bb595). It has a wall face's shape, as wide as the wall
sprite, and is read alongside the sprite with its read cursor starting
`1 << zoom` rows (2, 4 or 8, zoom counted from 1; 0x4bb4a2) before the
table, so its first row lies that many rows below the sprite's top and
the rows above take no pool. Each wall pixel
takes the drawn light byte plus each lamp's byte there, kept to 255, as
a floor does (0x4b7630 for one lamp, 0x4b7420 for two), and a room in
full light shows none. Diagonal walls take no pools (0x4c78c5 returns
none for them).

A corner's cap, the narrow slice of phantom wall face at an exposed wall
end, takes pools the same way (the cap pass reads the lamps at 0x53ca0a
and draws through the lit wall pass at 0x53ca41): the lamps of the tile
on its own seen side, along its own line, in the light of the wall it
takes its light from. Before, a lit wall's cap stayed in the room's
light, a dark seam at the wall's end (found in review; a lone wall with a
lamp beside it now lights its cap too).

**Checked.** The images' face lines up with our wall sprites' opaque
face exactly at all three zooms and both directions, with the first row
as many rows below the sprite's top as read. Compared with the original at the tick-570
world: far zoom at rotation 2 rose from 90.1% to 90.3% exact (non-green
87.9% to 88.2%); of the 1,250 pixels the pools change, 869 now match
exactly (1 did before) and the mean channel error fell from 26 to 1.1.
At rotation 1, walls up, 80.3% to 80.5% non-green; of 463 pixels
changed, 269 match (none did), mean error 16.8 to 1.7. Rotation 1 is
where turning the lamp record the other way with the view (0x4aedc2)
would show, so the turn is now checked for walls, and floors read the
same record the same way. The remaining changed pixels are mostly a
step off. Near and medium zoom show no lit wall and do not change.

**Grass** takes no pools: the lamp record is read (0x4aed23) only by
the floor draws (0x4adb48, 0x4af474), the wall pass's three wall calls
and a debug text, not by the grass.

**Not ported.** The wall blitter's second mode (0x4b780d, with
0x4c4a77 for unlit walls), which a caller's argument selects; our walls
draw by the first (0x4c4907). The original reads a record whose first
lamp is empty as having none even if its second is set (0x4aec97 counts,
then 0x4c78c5 reads from the first); ours never builds such a record.
