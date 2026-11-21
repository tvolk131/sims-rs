# Find Location For mode 5 applies its first draw and view turn to the first coordinate

*Decided 2026-11-21.*

## Context

`20261115-find-location-mode-5-random-start.md` ported Find Location For
(opcode 16, `0x44b3c8`) mode 5's random start (`0x434921` to `0x4349e5`),
but swapped the first and second coordinates throughout `random_start`,
`interior`, and `turn` (`crates/sims-vm/src/primitives/find_location.rs`).

The bug surfaced in the full 44,200-update Goth day (`goth-day-20261115`)
at update 41,445, where the raccoon (`obj 10`, at tile `(41, 21)`, fine
`(664, 344)`) runs global tree `419` (`Wander`, node 8: Find Location For
mode 5, flags `0x05`, from `obj 10`) to place its wander target (`obj 37`,
GUID `0x000007C4`). With identical `lot_rng` draws (`first = -1`,
`second = -3`), the original places `obj 37` at `(648, 312)` (tile
`(40, 19)` — starting at `(41, 19)` and stepping one tile west in the
spiral), whereas the port placed it at `(632, 344)` (tile `(39, 21)`).
That wrong tile caused the raccoon's walk and subsequent trash-can
interaction (`obj 32`, `obj 37`) to run 36 ticks early, producing the
divergence cluster at updates `41,858`–`41,892`, `42,000`, `42,724`–`43,003`,
`43,200`, `43,693`–`43,956`, and `44,200` (including Mortimer's motive
drift at `43,200`–`44,200`).

## Evidence in the original binary

In `Sims.exe`, a fine point (`cXObject + 0xf4`, search record `+ 4`, and
`0x4347f0` locals `[ebp - 0x48]`, `[ebp - 0x44]`) stores the second
coordinate (`position[1]`, `y`) at `+0` (`[ebp - 0x48]`) and the first
coordinate (`position[0]`, `x`) at `+4` (`[ebp - 0x44]`), whereas a tile
(`0x433067`, `0x434c98`) stores `x >> 4` in byte 0 and `y >> 4` in byte 1:

1. **Tile/point conversions (`0x433067`, `0x434c98`).**
   - `0x433067` reads `[ecx + 4]` (`x`) into `tile[0]` and `[ecx + 0]` (`y`)
     into `tile[1]`.
   - `0x434c98` reads `tile[0]` (`x`) into `[eax + 4]` and `tile[1]` (`y`)
     into `[eax + 0]`.
   - `0x4349e5`–`0x4349f7` reads `[ebp - 0x44]` into `tile[0]` (the first
     index into `rooms[level][x][y]` at `0x434a55`) and `[ebp - 0x48]` into
     `tile[1]` (the second index).
2. **Start of `(1, 1)` (`0x434ab3`–`0x434ad1`).** The first draw modulo
   `size` is written to `[ebp - 0x44]` (`x`), and the second draw modulo
   `size` is written to `[ebp - 0x48]` (`y`).
3. **Random walk (`0x434951`–`0x43499c`).** The first draw (`% 10 - 5`) is
   stored in `tile[0]` (`[ebp - 0x28]`), converted by `0x434c98` to
   `[eax + 4]`, and added to `[ebp - 0x44]` (`x`). The second draw is
   stored in `tile[1]` (`[ebp - 0x27]`), converted to `[eax + 0]`, and
   added to `[ebp - 0x48]` (`y`).
4. **Rotated lot bounds (`0x407704`, `0x434f58`, `0x4791a6`, `0x434cb7`).**
   `0x407704` maps `(tile_x, tile_y)` under view rotation 1 to
   `(63 - tile_y, tile_x)` (`x` in `far`, `y` in `near`), under rotation 2
   to `(far, far)`, and under rotation 3 to `(tile_y, 63 - tile_x)` (`x` in
   `near`, `y` in `far`). `0x4791a6` stores the `tile_x` range in `+0x70` /
   `+0x74` and the `tile_y` range in `+0x78` / `+0x7c`, which `0x434cb7`
   checks against `tile[0]` (`x`) and `tile[1]` (`y`).
5. **Inverse view turn (`0x4349c1`–`0x4349e0`).** `0x47bc1f` is called with
   first argument `&[ebp - 0x44]` (`x`), second argument `&[ebp - 0x48]`
   (`y`), and `(-rotation) & 3` turns — the same argument order already used
   by `look_towards.rs` (`turn_in_square([x, y], rotation)`) and the exact
   inverse of `interior(size, rotation)`.

## Decision

Fix the coordinate axis swap in `crates/sims-vm/src/primitives/find_location.rs`:

- In `Machine::random_start`, apply the first draw to `x` and the second to
  `y` in both the `[1, 1]` branch and the random-walk loop, and keep the
  `size < 3` guard on the walk branch while guarding `size <= 0` on the
  `[1, 1]` branch (matching `0x434ab3`, where a 1- or 2-tile lot skips the
  walk and lets the search return false normally).
- In `interior(size, rotation)`, return `[far, near]` for rotation 1 and
  `[near, far]` for rotation 3.
- In `turn([x, y], rotation)`, call `turn_in_square([x, y], (4 - i32::from(rotation & 3)) & 3)`.

Update unit tests in `find_location.rs` and `tests/interpreter/find_location.rs`
(including an asymmetric `[1, 1]` test on a 64-tile lot so `x` and `y`
cannot be swapped silently), assert `obj 37`'s `[648, 312, 1]` placement in
the fitted-frame Goth-day test (`boot.rs`), and re-pin `TICK_31478` and
`TICK_43200` (the default-frames run calls mode 5 after tick 20,186; all
earlier Goth pins and all Newbie/Old Town/UD2 pins are unchanged). In
`crates/sims-game/tests/sound.rs`, the changed post-20,186 default-frames
Goth day triggers a rejected tickle social whose FWAV requests
`vox_tickler_rejected`; the install's `SimsGeneratedHitSource.hot` ships
the `vox_tickler_rejected*.xa` samples (IDs 2670–2674) but defines the
event under the typo `vox_ticklee_rejected` (ID 2692), so `the_goths_sounds_play`
allows `"no event vox_tickler_rejected"` alongside the missing route events.
