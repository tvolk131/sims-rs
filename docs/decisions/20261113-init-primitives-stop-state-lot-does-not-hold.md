# Init primitives stop at state the lot does not hold

*Decided 2026-11-13.*

Test Object Type, Set To Next, Refresh, Play Sound Event and the stack
object's definition words (scope 21) are ported from the old prototype.
`Program` now keeps every OBJD as the 108 words the original holds after
registration: versions 136 and 137 become 138, and every record is cut or
zero-padded to 216 bytes (0x431759), so every scope 21 index under 108
reads. Definitions are listed in the file's native resource order, as the
catalog registers them; the prototype used physical file order.

Refresh mode 0 only redraws, so it does nothing here. Modes 1 and 2 mark
the object's room dirty, which makes the next room update rebuild the
room's score and lighting. Rooms are not modeled, so those modes fail
loudly unless the object is in no room (data word 29 is -5). This is now
the most common init failure on the Goth house. Play Sound Event only
queues audio, so it takes the true branch and changes nothing.

Set To Next walks registration order (modes 0, 2 and 6), people (1),
multi-tile groups (3) and families (12). Mode 4 fails loudly: the
original keeps a per-GUID list of instances for the whole process. The
list is filled in ID order on first use and appended to as objects
register, even across lot loads, and `World` has no such list. Modes 5
and 7 to 11, and unknown modes, also fail loudly. Three choices are
**unverified**:

- The person vector is the people in registration order. That is how
  loading builds it; Sims joining or leaving may change it.
- A multi-tile group's chain is its head, then its tiles in registration
  order, which is how loading links them (0x423223). `World` keeps only
  each tile's head, so splitting or joining groups at run time would need
  the links themselves.
- Test Object Type on a tile whose group has no catalog master, and Set
  To Next mode 6 in a tree with no parameters, fail loudly. What the
  original does there is not known.

The three load callbacks that fail with "tree 0x100b is missing" are not a
linking bug. `FlowersOutdoor.iff` OBJD 16808 has no OBJf, so its callbacks
come from the OBJD itself, and its load field names 0x100b. The file has
no such tree and no semi-global file. The prototype found that the
original skips a callback whose tree is absent (0x4749b5). Whatever runs
callbacks will have to do the same.
