# Wall caps follow the original's corner records

*Decided 2026-11-14.*

Walls in the original are thick, and their sprites stop at the corners,
so wherever a wall end shows, something must close it off. The original
does this in two places, and both are now drawn. Read from its code;
`crates/sims-render/src/caps.rs` follows it step by step.

**Corner records.** The original groups walls into straight runs. A run
ends wherever walls meet at an angle or a wall stops; those corners have
a record (0x53d2fc's tables). When the cutaway changes it works out, for
each record, which walls stand, which are cut, and which need a cap:

- A cut wall with no standing wall at the corner moves to the cut set.
  A cut wall that meets a standing wall is *marked* instead (bit 2 or 4
  of its flags, one per end). A cut wall next to a standing one along
  the same run is marked at that end too (0x54059a).
- Fences, and cut walls in a door-like style, take no trim or cap there.
  Marked walls take no trim either.
- The walls left over pick a cap from a table (0x53cd4e). A lone wall
  end, and an L-shaped corner whose outside faces the viewer, take one;
  a straight pass-through, or a corner opening toward the viewer, does
  not.

**The cap at a corner** (0x53c6b7) is a slice of a wall face, 3 pixels
wide at the far zoom (2 at some diagonal corners), doubling at each
zoom. It lies across the wall's end, beside the corner, in the plain
wall shape (style 255's for a wall of that style) and the pattern of the
wall it closes, lit like the wall at the corner that lies along it, if
any, else like the wall it closes. It stands on the corner's own ground,
as the original projects the corner's position.

**A marked cut wall** (0x53ea55) is drawn in wall shape 7 or 8 (by which
end is marked) instead of its cut shape. Those shapes stand full height
for a few pixels at the marked end. Over that, the original lays wall
pattern 252 or 253 in shape 9 or 10: a strip of wood trim down the
stub's edge. The strip is drawn at full light. With the wall's light,
the cutaway box scored 85.3% instead of 85.8%, and the trim along the
top is at full light too.

**Diagonal cut corners** (0x53c478). Where cut trim runs along a view
diagonal with bit 0x10 and a wall also leaves along +x (bit 8) or +y
(bit 4), the original lays the cut set's frame again over the right or
left part of the corner. This is drawn but changes no pixel in the
screenshots we have.

The trim at corner records now comes from the same records: standing
walls' trim, less fences, cut door-like walls and marked walls; the cut
walls' trim, less the same. At corners a run passes straight through,
the trim keeps the earlier rule (2026-11-14), since the run's own
drawing (0x53df0b) was not read in full.

**Result** (exact matches, house box and whole screen):

| Camera | Before | After |
|---|---|---|
| Medium zoom, rotation 0, walls up | 88.9% / 85.8% | 89.3% / 86.0% |
| Far zoom, rotation 0, walls up | 89.0% / 89.8% | 89.5% / 89.9% |
| Far zoom, rotation 3, walls up | 88.7% / 86.2% | 88.8% / 86.3% |
| Far zoom, rotation 0, cutaway | 83.8% / 88.4% | 85.8% / 88.8% |

The caps' column was checked one pixel either way at all four cameras;
the scores were the same or worse.

**Unverified:**

- The extra pieces at diagonal cut corners; no screenshot shows them.
- Caps on diagonal walls, and a marked diagonal wall's strip, which
  the original nudges at some zooms (0x53ebc0, not ported).
- Near zoom, and rotations 1 and 2.
- The original keeps one record for a run that is one segment long,
  so a mark made at one end is also seen at the other; our per-segment
  flags behave the same way, but no screenshot shows such a wall.
- Which room's light the cap takes when the corner has walls on more
  than one line; the original searches the corner's walls for one on
  the cap's line (0x53c97f), and so do we.
