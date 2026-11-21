# UserData1's House 69 matches the original's recording to update 1000

*Decided 2026-11-20.*

`an_old_town_home_loads_through_its_guid_table_and_steps` (in
`crates/sims-vm/tests/boot.rs`) loads House 69 of the first neighborhood
from the install and steps it 1,000 times. Track 2 recorded the original
doing the same (`house69-ud1-ticks-20261120`). The test takes three
things from that recording, none of which the house file holds:

- **The frames.** It draws the recording's frames
  (`UD1_RECORDED_FRAMES`): after updates 0 to 4, 32, 62, 92, 101 and 102,
  then every 30th from 128.
- **The frame arena's tree bits.** The original reads a saved frame's
  tree word into an arena it does not clear past four bytes (0x4750f0,
  0x4750fc), and keeps the bit 15 it finds there (0x4746e4). Dispatch
  tests that bit (0x474b99). On a set bit the object's hook (0x44aa96
  to 0x474b13) clears it and runs the same node again. At entry the
  recording has the bit on frame 1 of objects 263, 264, 265 and 268 (`UD1_FRAME_TREE_BITS`).
  `Lot::load_with_frame_tree_bits` takes those frames as an input. The
  bits are heap residue, like the residue bytes of
  `20261120-opened-neighborhood-boots-in-memory.md`, so they come from
  the recording, not from a port.
- **The dialogs' buttons.** Global 12 is the bitmap ID of the UI button
  last activated (0x4764fe to 0x46d9ce; the setter is 0x46d9c4).
  The fourth dialog after the load was closed by a button whose bitmap is
  17 (`WinBtn.bmp`). The original's modal loop (0x5dab4b) polls the dialog
  inside the lot update (0x4d1a8b), so that update ends with global 12 at
  17, and the next update's poll publishes 0. The other dialogs closed
  themselves and left it alone. `Lot::replay_dialogs` takes each answer's
  status and, where a button closed it, its bitmap (`UD1_DIALOGS`), and
  `Lot::step` publishes 0 before each lot update.

**Compared.** Track 3's `cmp_house.py` reads the tick, the C runtime, the
lot and global streams, the globals, and every object's data, temps,
attributes, last result, frames and person data. Relationships, which it
does not read, were compared on their own. It skips the words our port
marks unknown, where a script that reads one fails loudly:
- object 283's frame 2 local 7, which starts on half of a pointer, from
  update 14 to 320;
- person word 37, which the original wrote from memory it never set up
  (0x45c418): object 274's at times from update 241 to 712, and object
  283's from 321 to 665.

None of them is skipped at entry or at update 1000.

Our run matches the recording at entry and at every update through 1000
in everything else, relationships included, apart from:
- object 2's last result, from entry on (1 in the original, 0 here), as
  in UserData2's House 69;
- word 77 of object 274's frame block, its seventh frame's one local
  (tree 8292), from update 112 to 157 only: 3248 in the original, 0
  here. As in UserData2, nothing had written that word, and the original
  allocates the block without clearing it.

So `OLD_TOWN_69` and `OLD_TOWN_69_1000` are regression pins of our run.
At both points it differs from the recording, in what the comparison and
the relationship check read, only in object 2's last result.
