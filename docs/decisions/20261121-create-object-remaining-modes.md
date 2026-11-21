# Create New Object Instance remaining modes and flags (`0x44e0c8`)

## Context

`Create New Object Instance` (opcode 42, `0x44e0c8`) previously ported 8 of the 15 jump-table modes (`0, 2, 3, 4, 5, 6, 8, 9`) and a subset of flags per mode, trapping on modes `1, 7, 10..=14`, out-of-range modes (`> 14`), flags `0x02` and `0x40`, flags `0x08` and `0x20` on container/off-lot modes, and person/group targets in container modes.

## Behavior recovered from `Sims.exe`

Disassembly of `0x44e0c8..0x44e92e` (jump table at `0x44e92e`):

1. **All 15 jump-table modes (`0..=14`) and `> 14` fallthrough (`0x44e143..0x44e150`):**
   - Mode `1` (`0x44e215`): puts the new object into `self.me` container slot `0` (leaving `[ebp-0x2c]` at its initial `0`, identical to mode `2` at `0x44e22f`), with room filter `-5`.
   - Mode `7` (`0x44e3fd`): places where the object named by **parameter 0** stands (`cmp byte ptr [edi+7], 0; ja 0x44e418`; throws native error `8` if the frame has no parameters; returns false if parameter 0 names no object). Does not read `operands[6]`.
   - Modes `10` (`0x44e24d`) and `11` (`0x44e26e`): into `self.me` container slot `2` or `3`, keeping room filter `-5`.
   - Modes `12` (`0x44e28f`), `13` (`0x44e2ce`), and `14` (`0x44e30d`): into `stack_object` container slot `2`, `3`, or `4`, filtered to `stack_object.data[29]` (`0x44e3d9`).
   - Modes outside `0..=14` (`movsx eax, byte ptr [eax+4]; cmp eax, 0xe; ja 0x44e4c5`): jump straight to the common tail with the initial `[0, 0]` position and `place_on_lot = true`, which fails the off-lot test (`0x422632`) and returns false.
2. **Unified common tail (`0x44e4c5..0x44e91c`) and all flags:**
   - `0x01` (`0x44e4e1`) and `0x08` (`0x44e51e`): run in every mode, including container modes and mode 6.
   - `0x40` (`0x44e564..0x44e577`): resets the wall-check facing to `-1` and the room filter to `-5`, bypassing both the room check (`0x44e582`, modes `3, 4, 12..=14`) and the closed-direction wall check (`0x44e5ae`, mode `9`).
   - `0x20` (`0x44e640..0x44e6b4`): runs in every mode, including container modes.
   - `0x02` (`0x44e87c..0x44e8df`): after `"init visitor"`, if `0x10` is also set raises native error `0x36` (`0x44e88c`); otherwise, when the created object's stack is non-empty, writes `self.me` (`[esi+0xf0]`) into its bottom frame's first data word (`0x44e8d1`) and copies the caller's `stack_object` (before it becomes the new object at `0x44e90f`) into that frame's `stack_object` (`0x44e8db`).
   - `0x80`: unread in `0x44e0c8`.
   - Person and group creation in container modes go through the same `make_person` / `make` -> `fits_in` -> `put` (or `remove_now`) tail as ground modes; a multi-tile group is refused by `fits_in` (`0x422ac8`, code `0x10`) and removed.

## Verification

- Covered by unit tests in `crates/sims-vm/tests/interpreter/create.rs`, `visits.rs`, and `find_location.rs`: modes `1, 7, 10..=14`, out-of-range modes (`15, 255`), flags `0x02`, `0x02 | 0x10` (`0x36`), `0x08` and `0x20` on container/off-lot modes, flag `0x40` skipping room and wall checks, unused bit `0x80`, and person/group targets in container slots.
- **Unverified against a Windows recording**: none of the current Goth or expansion recordings execute modes `1, 7, 10..=14` or flags `0x02`/`0x40` yet.
