# Go To Routing Slot names its slot by parameter or from the lot's global slots

*Decided 2026-11-15.*

Operand word 1 of Go To Routing Slot picks the slot table (0x45ac22): 1
is the object's table by the literal index, as before; 0 is the
object's table by the index held in that parameter; 2 is the lot's
global routing slots by the literal index. The global table is the kind
3 entries of the global file's SLOT 100, which the original copies into
the lot (+0x9c) as it loads (0x440635, 0x4430e9). A global slot is still
placed relative to the stack object. Any other source, or an index past
the table, is native error 0x25.

The route's flag 0x4000 is written into the lot's single copy of a
global slot, so one object's route changes it for every object. It is
kept in `routing.slot_flags` under object ID 0, which no object has,
rather than in a new world field, so the world's encoding and hash stay
as they are. The original's bound check on a parameter number (0x45ac34)
never fails, and a number past the frame's parameters reads whatever
follows; that fails loudly here. Signal 0x10b clears the lot's table and
parses it again (0x443090 to 0x4430e9), restoring the file's flags; it is
not ported, and a port must drop the `(0, _)` entries.

A player's order for a snack reached source 2 at tick 154, in tree
0x2156; the Goth house's first 1000 ticks do not use it and are
unchanged. **Unverified** against a capture.
