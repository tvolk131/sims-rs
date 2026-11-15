# The door's left jamb is a depth contest, not a mask

*Decided 2026-11-14.*

At medium zoom our wall covers about a pixel column of the front door's
left jamb. Two findings, from the original's code:

- **The opening is masked the same way.** For door and window styles
  (22 and 23) the original finds the object on the wall's tile
  (0x43595b) and draws the wall in that object's opening shape: its
  wall shape plus 3 times the cut level plus zoom minus 1 (0x53d525).
  We do the same for standing walls; a cut wall at a door or window is
  drawn as a plain stub instead (the cutaway hides the door with it).
  The opening shapes carry no depth.
- **The jamb loses on depth.** In our depth space the wall is 4 bytes in
  front of the jamb's part at that column. The original's wall depth
  comes from per-zoom tables (a row curve plus a column table, about 6
  bytes per row and 4 per column at far zoom), compared `z <= prev + 1`
  in the wall blitter (0x4c4907), while sprites compare
  `candidate <= previous`. Ours has the same slopes but not the same
  rounding.

Tried and dropped, each within a few hundred pixels either way: moving
walls 4, 8 or 12 bytes back; pairing wall rows and columns one step
apart; a smooth per-column wall depth. A real fix is a port of the
original's depth tables at every zoom, which need reading from a running
game. Reopen with those tables.
