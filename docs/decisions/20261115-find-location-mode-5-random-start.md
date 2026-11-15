# Find Location For mode 5 starts each search from a random walk

*Decided 2026-11-15.*

Find Location For (opcode 16, 0x44b3c8) used to fail loudly in mode 5. A
survey of the 648 houses in the UserData neighbourhoods found 72 lots
stopping there in their load: House40 to House48 of each of the eight
neighbourhoods, where object 44's tree 0x1002 places object 45 in mode 5.

What was decided:

- **Mode 5 is mode 0 with a random start.** The code for mode 5 (0x44b57c
  to 0x44b5da) is mode 0's (0x44b5dc on) with one more store: byte 0x17 of
  the search record, which the record's constructor (0x4347d1) clears, is
  set to 1 (0x44b5a5). The rest is the same: a search in the start tile's
  room, then one anywhere, the place test again, the put with the lot told
  (virtual function 0x58), virtual function 0x74, and the true branch.
  The old comment that mode 5 is "the one that draws random numbers" was
  right, but nothing else about it differs.
- **The random start** (0x434921 to 0x4349e5, `Machine::random_start`). It
  runs in each of the two searches, from the same start point, so a failed
  first search draws a second start.
  - **A start of exactly (1, 1).** The start is replaced by two draws from
    the active random stream (0x46f457), each modulo the lot's size. The
    first draw is the second coordinate. They are read as points, not
    tiles (0x434ab3).
  - **Any other start.** Two draws modulo 10, less 5, are added to the
    point as tile offsets, each to the middle of its tile (`(n << 4) | 8`,
    0x434c98); the first draw goes to the second coordinate. This repeats,
    adding to the point already moved, until the point's tile is inside
    the lot's bounds less one tile at each side (0x434cb7). The tile is
    each coordinate shifted right 4 and cut to a byte (0x433067). On the
    six-tile test lot that cut matters: a walk that wanders off returns
    after it wraps.
  - **Either way**, the point is then turned within a square of 1024
    (0x47bc1f, size 0x400) by (-rotation) & 3, where the rotation is the
    world's +0x84 (`Interpreter::view_rotation`). The spiral then starts
    from the turned point, clamped into the lot as in mode 0.
- **The bounds.** They are the world's +0x70 to +0x7c, the box round the
  lot's four corner tiles, each turned by the world's rotation. The
  rotation (0x4791a6) and the resize (0x477fbb) set them, and the turn
  (0x434f58) looks each corner up in a table (filled at 0x407704) that
  turns tiles within the whole 64-tile square, not within the lot: a
  turn of 1 takes tile (a, b), as 0x433067 cuts it, to (63 - b, a). So
  on a lot of 64 tiles the walk's tiles must be 1 to 62 whatever the
  rotation, but on a smaller lot a turned view puts the bounds on the
  square's far side (tiles 65 - size to 62) along the point's first
  coordinate at views 2 and 3, and along its second at views 1 and 2.
  The walk still starts from the unturned point, so it goes round,
  through the cut to a byte, until it reaches them, and only then is the
  point turned back. A first version took the bounds as 1 to size - 2
  whatever the rotation; the review found it from the table.
  The "scene" at 0x6e5528 is the same object as the lot's world at
  0x6e552c: 0x415c65 stores one new 0x88-byte object (0x478fef) in both
  the lot's +0x18 and +0x1c, which 0x4142d9 and 0x4142e4 copy to the two
  globals.
- **What fails.** A lot under three tiles has no tile inside its bounds, so
  the original's loop never ends, and a lot of size 0 would divide by
  zero. Both fail loudly, and so does a turned lot wider than 64 tiles,
  whose corners would be looked up past the table. Mode 2 still fails
  loudly.

Results:

- **The 72 lots.** They now get past mode 5, and all of them stop in the same
  place in the load: the search reads the room map (load step 8, while
  the lot-wide controllers are made) before the load has worked the rooms
  out (`World::rebuild_rooms`). That is the bare lot's room map, which the
  port does not model (`Interpreter::resize`).
- **Strict runs.** The Goth and Newbie strict runs are unchanged, since no
  object of theirs uses mode 5.

The random start's rotation depends on the player's camera, which makes the camera
direction an input to the simulation here, as it already is for Generic
Sims Call routine 12.
