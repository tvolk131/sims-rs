# A sound's volume and pan come from where its objects are on the screen

*Decided 2026-11-15.*

Kinds 1 (for a `kArgsIdVolPan` track) and 30 now set the track's
`extvol` and `pan` from every object recorded for it, as the original
does after starting it (0x573795 to 0x5737a4, 0x573d5d to 0x573d6e), in
`sims_audio::placement`. Each object is placed by 0x5730b6, and the sum
(0x57473b) takes the loudest volume and the volume-weighted pan, in
32-bit arithmetic that wraps.

The placement uses the screen when the track's `Spl` (register 57) is
not 0. `Spl` starts at 20, `spl_normal`, from the default bank for
registers 50 to 62 (0x6f6898, filled at 0x57c83a to 0x57c97b), and no
script in the install writes it, so every placed object's volume and pan
come from the screen: the pan from how far across it the object is, the
volume from the zoom (the far zoom takes 560 of 1024 away, the medium
280), less off the middle of the screen, and three fifths of that on a
floor the view does not show (`placement::project`). The game sends
each object's position (`sims_sim::Object::position`, the original's
+0xf8, +0xf4 and +0x110; `Snapshot::positions`) and its place on the
screen with the zoom, the screen's size and the floor shown
(`Snapshot::screen`, from `sims_game::sound::screen`), again whenever
the camera changes. The place on the screen is the original's (0x570e09):
the position rotated into the view about 1023, not the drawing's 1024,
then projected as the original's view does it (0x4a9dc7,
`sims_game::sound::fine_point`): the tile's corner, then each part of
the step across the tile cut toward zero on its own, in 16 bits. A point
can be a pixel off the drawing's (`project_fine`). The original also
raises the point, and slants the step, by the heights of the tile's
corners, which is left out (**unverified** on a lot that is not flat).
Without a place on the screen the placement is a fault.

The same bank corrects three defaults the player had wrong: register 39
(`BufSizeMult`) starts at 1 and 52 (`tempo`) at 120, and 40
(`fade_dest`) at 0, not 1. So a `seqgroup_kill` (0x57eef3), which fades
the holder without setting `fade_dest`, now fades it to 0, not 1.

What the original does with an object it cannot place is kept: an ID
that names no object, or an object with a first or second coordinate
of 0, leaves the previous object's values, and the first object's are
pan 0 and a volume a local held before: the address of the sound
manager's instance map (0x576235), on the heap. Here that address is
0x10000d4, **unverified**. It outweighs the other objects in the pan,
and both the pan and the volume come out where the 32-bit arithmetic
wraps them: with this address and a `vol` of 1024 the track plays at
212 of 1024, not at full. A `kArgsIdVolPan`
kind 1 with object 0 now starts nothing (0x57375b), where it started
the track; kind 30 records object 0 as an instance, where it skipped it.
The writes leave the comparison flags the pan's.

The volume and pan are worked out when the event comes, as the original
does, so `Player::handle` and `Player::play_from` now take the game's
`Sources`. A track's instances go in the original's multimap's order
(0x5761c2): the first recorded, then the others newest first. The
order the map's growth gives them (0x5762c7) is not ported; it only
matters for an object that cannot be placed.

Not ported: placing every track again on kind 33 (0x57447f), which the
original's game sends when the camera moves. The game here sends kind
33 every frame, and placing again would undo a fading `extvol` each
frame. Also not ported: the limit of 16 voices that kind 1 checks first
(0x574a87).

**Unverified:** nothing of this has been compared with the running game.
