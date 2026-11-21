# The rooms' cutaway masks are kept from the zoom they were built at

*Decided 2026-11-20.*

This settles the open question in "The strip on a marked edge-on diagonal
is ported" (2026-11-20): at medium and near zoom our cutaway mask held a
row of tiles more than the original's. The original's mask was read, from
its code and from the running game, not fitted.

What the original does:

- Each room keeps its own part of the mask, 64 by 64 view tiles at the
  room's +0x80, built by a room method (0x468203). The view's mask only
  combines the kept parts of the rooms it looks at (0x4ab992, at
  0x4abafe). The squares an outdoor room adds (the pointer's, the
  selected Sim's) are worked out there, at the zoom shown.
- The parts are built at the zoom of the moment, for every room of one
  level or of all levels (0x466f0c), when:
  - the view turns (0x4a7f28, 0x4a8006), for the floor shown;
  - the floor shown changes (0x4a8116), for the new floor;
  - a room update ends (0x466c3c, at 0x466ef6), for the levels it ran
    for, whether or not it counted a room; the update runs from the
    frame's draw when rooms wait (0x467094, from 0x4a8895), and at load;
  - message 0x85 arrives (0x41648c), for every level. Its senders were
    not read.
- A change of zoom (0x4a7c9f) builds none: it calls 0x4ab992 only. A
  house opens at far zoom (0x4a6c5f), after its load's room update.
- Built at another zoom, a part can differ by a row of tiles. The
  builder's overlap test puts the room tile's lower edge at its top
  corner plus half a tile less 2 (0x46831c), and that 2 does not scale:
  the next row down the screen just touches it at far zoom and is left
  out, but overlaps it by 2 pixels at medium and by 6 at near. For
  House02's room 2 that is 18 tiles.
- 0x466f0c skips a room whose +0x34 is 0. In the running original every
  room of the Goth house had it set to 1; what clears it was not read.

Seen in the running original (the Goth house):

- Paused, zooming kept every room's part byte for byte. Turning the view
  away and back at medium changed them, and rooms 1 to 4 then differ by
  the zoom they were built at; the outdoor rooms 1025 and 1026 do not.
- Running, room 4 was built again at the zoom shown within about 3
  seconds of unpausing, and a change of zoom was followed within about
  2 seconds: rooms are marked often in play (a room whose light changes
  is marked again), and each update builds its level's parts.
- The view's tile sizes and storey heights equal ours, as recorded
  before.

How it is modeled:

- `Focus::masks_built_at` is the zoom the parts were built at, or None
  for the zoom shown. `Mask::new` builds the looked-at rooms' and the
  selected Sim's parts at that zoom, and the squares at the zoom shown.
  One zoom stands for all the rooms of the floor shown, because every
  build covers all the rooms of a level and the mask uses only the floor
  shown.
- The game's view keeps it: a change of zoom keeps the zoom the parts
  were built at (None once back at it), and a turn or a change of floor
  sets None.
- `Interpreter::masks_rebuilt` notes the levels each room update ran for;
  `Lot`'s steps report them in `Update::masks_rebuilt`, and the game sets
  None when the floor shown is among them. That is the only place ours
  runs a room update after the load.
- A house opens with None, at far zoom, as the original's does.

Measured against the House02 medium and near shots, which were taken
paused after zooming in from far (focus on rooms 2, 1 and 3; doors and
windows given their hide-with-walls flag): exact pixels (16-bit) go from
317380 to 317864 of 336000 at medium and from 270154 to 271711 at near.
Where the two masks differ, the kept one matches the original on 484 of
662 pixels at medium and 1602 of 2236 at near; the one built at the zoom
shown matches on 0 and 45. The strip on (26,23) is then drawn where the
original draws it at every zoom. The House02 test pins it at medium.

The Goth-house ratchet scenes set the camera directly, with masks at the
zoom shown, as before.

Not modeled:

- Message 0x85, whose senders were not read.
- Rooms whose +0x34 is 0, which a build skips: none was seen.
