# Road centre-line dashes swap on odd-parity camera turns

*Decided 2026-11-21.*

At rotation 1 in `far-r1-cut-0816`, the road's white centre-line dashes drew perpendicular to the street, forming short crossbars instead of a dashed line along the pavement.

## What the original does

Read from the Complete Collection 1.91 executable (`Sims.exe`):

- **Floor sprites have a single orientation.** `GameData/floors.iff` holds one frame per zoom for every floor ID; `SPR2` 10/266/522 and 11/267/523 are the two perpendicular orientations of the dashed road tile.
- **In-place swap on odd-parity camera turns (`0x4a7dfe`, `0x4a8042`–`0x4a80b1`).** When the view's rotation setter is called with its second parameter (`force_no_swap`, `[ebp + 0xc]`) equal to `0`, it computes `abs(old_rotation - new_rotation) & 1` (`0x4a8047`–`0x4a8059`). When that is `1`, it walks `y = 0..lot.size`, `x = 0..lot.size` (`0x4a8063`–`0x4a80b1`) over level 1's floor array (`[lot + 0x24]`, set to `floors` at `0x478b44` and indexed as `[lot + level*4 + 0x20]` at `0x4aef44`) and replaces every cell holding `10` (`0x0a`) or `11` (`0x0b`) with `id ^ 1` (`0x4a808d`–`0x4a809e`).
- **Level 2 and split-tile wall floors are untouched.** Only `[lot + 0x24]` (level 1) is scanned, not `[lot + 0x28]` (level 2), and split tiles store `255` (`0xff`) in the floor array while keeping their two halves' floor IDs in the wall array, so `cmp ax, 0xa` / `cmp ax, 0xb` skips them.
- **House loads keep the saved orientation (`0x4a861f`, `0x4a87b1`–`0x4a87dc`).** Loading reads `HOUS 0`'s `camera_direction` into `[0x6f00dc]` and calls `0x4a7dfe([0x6f00dc], 1, 1)` (`force_no_swap = 1`). No save or load path un-swaps the on-disk floor chunk (`Arry 1`, or `Arry 11` from house version 63 per `0x478617`–`0x478652`, including `House24.iff` and `House62.iff`), so a house saved at an odd rotation keeps its swapped `10`/`11` IDs on disk. Checked across active cells (`0..lot.size`) of `UserData/Houses/House*.iff` in Complete Collection: every `const-x` (Y-axis) road lot saved at even `camera_direction` (`0`/`2`, e.g. `House05.iff`) stores `11`, while those saved at odd `camera_direction` (`1`/`3`, e.g. `House24.iff`, `House51.iff`, `House59.iff`) store `10`; `const-y` (X-axis) road lots follow the exact converse (`10` at even, `11` at odd, e.g. `House02.iff` vs `House62.iff`). Inactive cells outside `0..lot.size` (such as `y = 60` in the 50×50 lots `House48.iff` and `House89.iff`) are never visited by `0x4a8063`.

## How `sims-render`, `sims-viewer`, `sims-vm` and `sims-game` model it

`sims-render::draw` does not mutate `World` or `Lot` when the camera turns:

- `sims_render::Lot` records `floor_rotation: u8` (`0` by default in `Lot::from_world`, set via `Lot::with_floor_rotation` from `HOUS 0`'s `camera_direction & 3`), naming the camera rotation that `levels[0].floors` is oriented for.
- `Painter::floors` maps each non-split level-1 floor ID `10` or `11` through `id ^ 1` whenever `(camera.rotation ^ lot.floor_rotation) & 1 != 0`. Level 2 and `SPLIT_FLOOR` halves are left unchanged.
- `sims-viewer` passes `opened.rotation` to `Lot::with_floor_rotation` so odd-saved houses open without swapping. For `--snapshot`, `sims boot --snapshot` records `lot.floor_rotation()` inside the snapshot record itself via `sims_sim::Snapshots::save_with_floor_rotation`, and `sims-viewer` reads it back with `Snapshots::load_with_floor_rotation` (falling back to `opened.rotation` only for older snapshots that recorded none), so viewing one house's boot snapshot with another house's art keeps the snapshot's road-dash orientation across all four rotations and overwriting a snapshot always replaces its rotation metadata atomically.
- `sims-vm::boot::Lot` keeps `floor_rotation` from `opening_rotation(&file)` (unchanged when `set_view_rotation` turns the camera), persists it in `Saved` (bumping `sims_vm::boot::save::FORMAT` from `3` to `4`), and rejects restoring a snapshot whose `floor_rotation` differs from the loaded house's (`SaveError::OtherLoad`). `sims-game::Game` reads `self.lot.floor_rotation()` whenever `lot_shape` is built (open, tick refresh, and snapshot restore).

## Measured ratchet deltas (vs `f2de143f`)

Only `far-r1-cut-0816` changes; all other 7 scenes keep identical frame SHA-256 hashes:

| Scene | `frame` | `surface` | `objects` | `people` | `panel` |
|---|---:|---:|---:|---:|---:|
| `far-r1-cut-0816` | 99.23% → 99.46% (+0.23 pp) | 99.32% → 99.51% (+0.19 pp) | 99.20% → 99.20% | 5.61% → 5.61% | 98.97% → 99.88% (+0.91 pp) |

The `panel` jump comes from the road passing behind the transparent gaps around the control panel's curved edge.

## Note on the corner kitchen counter (`near-r3-down-0816`)

Investigation of the `near-r3-down-0816` `objects` outlier (`94.46%`) showed that counter ID `117` (`0x91767a36` at tile `(16, 30)`) is saved in `House05.iff` `ObjM` with `data[0] = 1` (corner `DGRP 101`) and `attrs[5] = 5`, but is reset on load by `sims-vm` because `Tiled Counter`'s linked script signatures differ from the save, leaving `data[0] = 0` (straight `DGRP 100`) in `World` before `sims-render` ever sees it. That is a `sims-vm` simulation-state issue rather than a renderer defect, so it is left out of this change.
