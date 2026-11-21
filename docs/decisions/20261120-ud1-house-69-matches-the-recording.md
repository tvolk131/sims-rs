# UserData1's House 69 runs 1,000 updates against the original's recording

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
  inside the lot update, so that update ends with global 12 at 17. The
  application's next poll (0x4d1a8b) publishes 0. While the other
  dialogs were open, global 12 did not change.
  - `Lot::replay_dialogs` takes each answer's status and what the wait
    check published while it was open (`UD1_DIALOGS`): 17 for the
    fourth, nothing for the others.
  - `Lot::step` publishes 0 before each lot update. The original polls
    once per pass of its loop, and that pass's idle call can run several
    updates (0x46da70 to 0x46dac7). That every update here follows a
    poll is unverified. The recording, where global 12 is 0 again at
    update 4, does not tell the two apart.

**Compared.** Track 3's `cmp_house.py` reads the tick, the C runtime, the
lot and global streams, the globals, and every object's data, temps,
attributes, last result, frames and person data. It does not read the
objects' footprints, and object 385's differs at entry and every
update: `[-20, -20, -12, -12]` in the original (object +0x100,
0x433e9b) and `[-24, -24, -8, -8]` here. That is not explained yet. The
frames drawn between updates are the recording's; the recording's draws
inside updates are not a separate input. Relationships, which it
does not read, were compared on their own. It skips the words our port
marks unknown, where a script that reads one fails loudly:
- object 283's frame 2 local 7, which starts on half of a pointer, from
  update 14 to 320;
- person word 37, which the original wrote from memory it never set up
  (0x45c418): object 274's at times from update 241 to 712, and object
  283's from 321 to 665.

None of them is skipped at entry or at update 1000.

Our run matches the recording at entry and at every update through 1000
in everything else those two read, apart from:
- object 2's last result, from entry on (1 in the original, 0 here), as
  in UserData2's House 69;
- word 77 of object 274's frame block, its seventh frame's one local
  (tree 8292), from update 112 to 157 only: 3248 in the original, 0
  here. As in UserData2, nothing had written that word, and the original
  allocates the block without clearing it.

So `OLD_TOWN_69` and `OLD_TOWN_69_1000` are regression pins of our run.
At both points it differs from the recording, in what the comparison and
the relationship check read, only in object 2's last result. Object 385's
footprint, which neither reads, differs as well.

**Residue read as zero.** The words of a frame block that nothing wrote
are 0 here, as object 274's word 77 shows. A script that read one
before writing it would branch on 0 where the original has its heap's
leftover. They are not yet marked unknown.
