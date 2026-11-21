# The void off the lot follows the time of day and house 99

*Decided 2026-11-21.*

The screenshot ratchet (`docs/decisions/20261120-screenshot-ratchet.md`)
recorded that in the night scene (`night-far-r0-cut-2153`) our void off
the lot stayed the daytime grey while the original drew it dark. The
original's clear-colour function was read from `Sims.exe` rather than
fitted to the screenshot.

## What the original does

When the view clears its colour buffer (`0x4aaa26`, called from
`0x4ad12e`, `0x4ad181` and `0x4ad267` beside the depth clear `0x4aaa97`),
it calls `0x4aa9bd` for the 16-bit fill colour:

- If the view's override flag (`view+0x17c`) is non-zero, it returns the
  saved 16-bit colour at `view+0x182`. That override is set by `0x4b2b1a`
  (at `0x4b2e8d`, cleared at `0x4b3037`) for off-screen lot snapshots and
  is 0 during normal lot drawing (cleared in the view constructor at
  `0x4a6d3a`).
- Otherwise it reads the globals block (`[[0x6e48bc]+0x1c]`), starting
  with fallback `0x0000ff`:
  - If global 10 (`+0x24`, the house number) is `99` (`0x63`, the cloud
    lot), it takes `0x14148c` regardless of the time of day.
  - Otherwise it switches on global 4 (`+0x18`, `time_of_day`):
    - `0` (day): `0x737373`
    - `1` (dusk) or `3` (dawn): `0x404040`
    - `2` (night): `0x1a1a1a`
    - any other value: keeps the fallback `0x0000ff`.
- It packs that `0x00RRGGBB` value into 15-bit 5-5-5 (`0x49f087`) when
  `[[0x6e48bc]+0x34] == 1` and into 16-bit 5-6-5 (`0x49f0ae`) otherwise.
  In 5-6-5, widened back to 8 bits a channel:
  - day (`0x737373`) -> `[115, 113, 115]` (5-6-5: 14, 28, 14)
  - dusk / dawn (`0x404040`) -> `[66, 65, 66]` (5-6-5: 8, 16, 8)
  - night (`0x1a1a1a`) -> `[24, 24, 24]` (5-6-5: 3, 6, 3) — dark grey,
    not pure black, matching the `night-far-r0-cut-2153` screenshot
    (`(24, 24, 24)` across the off-lot corners)
  - house 99 (`0x14148c`) -> `[16, 20, 140]` (5-6-5: 2, 5, 17)
  - fallback (`0x0000ff`) -> `[0, 0, 255]` (5-6-5: 0, 0, 31)

## How it is modeled

- `sims_render::scene::background(time_of_day, house)` implements
  `0x4aa9bd`'s selection and passes the result through
  `canvas::quantize565` (matching `0x49f0ae` and keeping alpha-blending
  against the void in the original's 16-bit values).
  `scene::BACKGROUND` remains `[115, 113, 115]` (the daytime colour).
- `scene::paint` reads live globals 4 (`time_of_day`, falling back to
  `lot.time_of_day` when `world.globals` lacks index 4) and 10 (`house`,
  defaulting to `0`) directly from `world.globals` on every clear, so
  changes to globals between ticks never leave a stale void colour.

## Measured

Screenshot ratchet (`crates/sims-game/tests/ratchet.txt`):

- `night-far-r0-cut-2153`: `frame` **88.48% -> 98.00%** (+9.52 pp),
  `surface` **88.41% -> 99.78%** (+11.37 pp); `objects` (74.18%) and
  `panel` (96.97%) unchanged.
- All seven daytime scenes keep their exact frame SHA-256 digests and
  floors unchanged.

## Not modeled

- The off-screen snapshot override (`view+0x17c` / `view+0x182`, set by
  `0x4b2b1a`): we have no off-screen thumbnail pass yet.
- 15-bit 5-5-5 display mode (`0x49f087` when `[[0x6e48bc]+0x34] == 1`):
  the renderer targets the original's 16-bit 5-6-5 mode everywhere.
- Dusk, dawn, and house 99 void colours are verified from `Sims.exe`
  (`0x4aa9bd`) and unit/render tests, not yet against captured
  screenshots of those states.
