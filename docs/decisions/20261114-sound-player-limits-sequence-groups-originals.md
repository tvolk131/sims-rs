# Sound player limits, and sequence groups from the original's code

*Decided 2026-11-14.*

A review of `sims-audio` found seven defects. The fixes:

- **Limits.** One block runs at most 100,000 instructions across all
  threads, and one thread at most 10,000. A thread started by
  `play_trk` first runs in the next block, so a track that starts itself
  cannot keep a block going, and the threads a full block does not reach
  run first in the next. At most 256 threads, 1,024 notes, 256 sequence
  groups and 1,000 recorded faults exist at once, and calls nest at most
  64 deep. A patch no table defines is not cached, since any number can
  name one. These numbers are ours. The original reports an error after
  101 instructions without a wait and carries on.
- **Arithmetic and sizes.** Mixing and interpolation use 64 bits. An XA
  header's claimed size is checked against what its data can hold before
  any memory is reserved. FSC sizes use checked multiplication. Hit-list
  files may expand to at most 100,000 entries (the largest in the
  install has 3,661); past that, parsing fails. Writing a WAV whose
  sizes do not fit 32 bits is an error, not an overflow.
- **Unported instructions fault.** `seqgroup_wait`, `add_to_group`,
  `remove_from_group` and `test1` to `test4` end their thread with a
  fault. The original does nothing for the first three, and no program in
  the install uses them; `test1` to `test4` are debug routines.
- **Sequence groups are ported, not faulted.** The review asked for
  `seqgroup_kill` and `seqgroup_return` to fault too, but 2,731 of the
  4,494 event definitions reach `seqgroup_kill` (most voices), so a fault
  would silence them. Instead they follow the original's code, read
  (never written) from a memory image of the running game: a group slot
  holds the ID of the thread that last claimed it.
  - `seqgroup_return` claims the group and ends its own thread if another
    held it.
  - `seqgroup_kill` claims it; another holder ends at once when its
    `extvol` is 0, and otherwise fades `extvol` toward `fade_dest` at
    speed 200 while the caller waits 2 ms and runs the instruction again.
  - `seqgroup_trkid` reads a group's holder.
  - Slots are not cleared when a thread ends, so a stale ID still makes
    a later `seqgroup_return` end. Only `StopAll` clears them here.
  - New threads start with the original's default register bank
    (`extvol` 1024, `fade_dest` 1, `pan` 512, `pitch` 3600 and others).
  - A fade adds in 64 bits, so it stops at its destination where the
    original's 32-bit sum could wrap past it.
  - **Unverified:** none of this was observed at run time, and how often
    the original updates fades is not known (here once per block). Volume
    is not applied yet, so a fade changes registers, not the sound.
- **Every definition is tested.** The install test runs each event
  definition as its own module defines it, not by first match. A
  separate test pins that 13 names are defined by more than one module
  (5 of them differently), and that the module whose name sorts first
  wins. How the original chooses is unknown.

Reopen when a run with sound shows different group or fade behaviour.
