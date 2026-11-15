# Held objects stand on their holder's slot

*Decided 2026-11-14.*

The lamps were missing because we drew them on the floor. The Goth
house has three, all table lamps: one on a kitchen counter and two on
end tables. Each is held by its table: data word 2 names the holder and
word 3 which of its slots. Their own position is the table's, and their
sprites carry no height, so they ended up inside the table and were
hidden by it. The sink in the other counter is held the same way.

The original draws a holder's contents while it draws the holder
(0x437fdd). It adds the slot's offset, turned by the holder's facing,
to the position, and adds the slot's height. We now do the same:

- the slots are the holder's SLOT entries of kind 0 (OBJD word 20
  names the chunk);
- the height comes from the entry's category, as the old prototype read
  it and checked against the original (`containment_height`): counter
  4.05, table and end table 4, low table 2.5, category 1 none, others
  the entry's own z;
- the height adds to the sprite's own height, which raises it on screen
  and moves its depth;
- the offset adds to the held object's own position and level, which
  the game keeps at its holder's (the original starts from the
  holder's; the two agree in this house);
- a hidden holder hides what it holds;
- a loop of holders gives no offset, and offsets and heights are
  clamped to a lot's width, so a broken save cannot overflow anything;
- objects a Sim holds stay where they were.

**Result.** At far zoom, rotation 3, the end-table lamp now matches the
original pixel for pixel in place. Exact matches in a box around it
rose from 62.2% to 78.1% (96.4% within one light step). The house box
rose from 88.5% to 88.7%. The other compared cameras do not show a
lamp: the lamps stand behind walls there.

**Unverified:**
- The kitchen counter lamp is not in view at any compared camera.
- Every slot here has no x or y offset, so the direction we turn slot
  offsets (the same as draw-group offsets) is not checked.
- The original draws held objects right after their holder; we sort
  them by depth with everything else, which puts them after it here.
