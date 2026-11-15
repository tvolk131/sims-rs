# The sound tables read, and an event leads to its samples (track 3, sound)

*Decided 2026-11-14.*

`sims-audio` now reads the sound scripts' files. A *module* is a HIT
program (bytecode) with its HSM file (symbol names) and HOT file
(tables). The install has 27 modules: two in the base game, one per
expansion pack, and one per downloaded object that has its own sounds.
The HOT tables map event names such as `Phone_ring` to tracks, tracks to
HIT entry points and patches (sample files), and list the hit lists
that scripts pick samples from. FSC files are the outdoor ambient
scores, and all four parse.

How the HIT bytecode is laid out:

- Opcode names and numbers come from the HSM files. Operand layouts come
  from public notes (the Niotso wiki, FreeSO's HIT player).
- The install confirms them for every opcode it uses. All 27 programs
  decode from their first byte to their last, and every one of the 4,363
  HOT entry points lands on an instruction.
- Branch, call and jump operands are not addresses. They are track data
  IDs that the HOT file maps to addresses; every such operand in the
  install resolves within its own module.
- `jump` has two forms: a 4-byte ID, or a 1-byte register holding the
  target. The opcode does not say which. We take the ID form when the 4
  bytes are an ID of the program. That covers 399 of the 401 jumps; the
  other two only decode as register jumps. **Unverified** how the
  original decides. Opcodes the install never uses keep the public
  layouts, also **unverified**.

What the files themselves leave dangling:

- 155 patches name sample files the install lacks. One path names a
  build machine's `c:\` drive.
- 14 events give an empty track.
- 10 track entry points use a symbol that their own module's HSM file
  does not define (`tkd_Generic`, `tkd_GenericInterrupt`). Other modules
  do define them, so the original may look across modules.
  **Unverified.**
- 4 tracks name a patch that does not exist.
- 6 hit list items are neither a number nor a range: four names and two
  dotted pairs such as `2370.2376`. These are kept apart, unread.
- The FSC scores mostly use samples in their own directory. The two big
  scores also use 56 samples from the other ambient directories, and
  six samples exist nowhere. Where the original looks for them is
  **unverified**.

The gated test pins all of these counts, so any change to the rules
shows up in it.

Traced by hand, then pinned in a test:

- **Phone ring.** `Phone_ring` plays track 3479. That track runs the
  generic routine (`note_on v1; wait_samp; end`) with the patch
  `sounddata\phone\Phone_ring.xa`.
- **Footstep.** `footstep_soft` runs a routine that sets `interrupt`,
  compares 2 with the game speed (global register 100), and branches to
  code that picks one of five soft footstep samples (hit list 217).

When two modules define the same event, the module whose name sorts
first wins (both events above are in the base game's generated module).
The original's order is **unverified**.
