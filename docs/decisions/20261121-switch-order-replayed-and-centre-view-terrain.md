# Portrait centring uses terrain heights, multi-tile averaging, and the 500-pixel house view; the switch order replays to update 900

*Decided 2026-11-21.*

Replaying track 2's switch-and-order recording of the Goth house
(`goth-switch-cancel-order-20261115`, 900 updates) through the window's
input layer uncovered two differences in how `Game::centre_on` and
`sound::fine_point` placed a centred Sim compared to `Sims.exe`. Review
also identified that `Game::centre_on` omitted `0x4071a3`'s multi-tile
part averaging.

## What the original does

- **Multi-tile object position averaging** (`0x4071a3`, called by
  `0x434ced`). For a multi-tile object (`cXObjectMultiTile`), `0x4071a3`
  steps to the lead tile (`+0x190`, or the object itself if null), walks
  the linked parts via `+0x18c` (`next_parts`), and averages their `x`
  and `y` positions by signed integer division toward zero.
- **The house view sits above the 100-pixel live-mode strip**
  (`0x49ae5e` → `0x5cf1b6`, `0x4a70ba`, `0x4b136b`, `0x53b2eb`). At
  800×600 `0x49ae5e` sizes the house view's viewport to `(800, 600 - 100)`
  and copies that rect `(0, 0, 800, 500)` into `HouseView + 0x28..+0x34`.
  `0x4a7777` and `0x4a7996` therefore centre on `[width / 2, (height - 100) / 2]`
  (`[400, 250]` at 800×600), not `[400, 300]`.
- **Fine-point projection includes terrain heights** (`0x4a9dc7`,
  `0x4a9c50`, `0x479567`, corner table `0x6c4ebc`). For a tile strictly
  inside the active lot's rotated bounds (`0x4791a6`), `0x4a9c50` raises
  the tile's top corner by `corners[1] << (zoom + 1)` and `0x4a9dc7`
  slants the sub-tile step using all four rotated corner heights. Both
  `0x434ced` (centring) and `0x570e09` (sound placement) call `0x4a9dc7`.
  On House 05's raised plateau (height 6), this lifts Bella's screen
  point by 24 pixels at far zoom; combined with the 50-pixel viewport
  centre shift (`250` vs `300`), the net camera shift is `-26` pixels in y
  (`origin = [472, -117]` instead of `[472, -91]`).
- **Scroll limits and x-alignment** (`0x4a9fa9`, `0x4b0615`, `0x4a81cc`,
  `0x4a7af0`). `0x4a7777` and `0x4a7996` clamp the target origin to
  `0x4a9fa9`'s scroll limits (using the rotated diagonal height extrema
  built by `0x4b0615` and the active lot's rotated tile bounds from
  `0x4791a6`) and commit through `0x4a81cc`, which aligns `origin.x` via
  `(x + 1) & !3`. The non-person / other-floor path (`0x4a7996`) checks
  the rotated active tile bounds via `0x4a7af0`.

## Ported

- `sound::fine_point_with_heights`, `sound::tile_corners`, and
  `sound::object_point_on_lot` port `0x4a9dc7` / `0x4a9c50` / `0x479567`
  with terrain heights; `sound::screen` (`0x570e09`) now uses
  `object_point_on_lot`.
- `Game::centre_on` (`0x434ced`) averages multi-tile parts' positions
  (`0x4071a3`), centres on `[width / 2, (height - 100) / 2]`, projects
  with terrain heights, checks `0x4a7af0`'s rotated active lot bounds on
  the `0x4a7996` path, clamps to `0x4a9fa9`'s scroll limits, and applies
  `0x4a81cc`'s `(x + 1) & !3` alignment.
- `View::apply(Move::Scroll)` and the initial `View::new` camera are
  unchanged (scroll limits apply to `Game::centre_on` here; porting
  scroll clamping to manual scrolling and loading the saved `HOUS`
  camera remain separate).

## Verified

- An offline comparison (scratch tools, not in the repository) replayed
  `goth-switch-cancel-order-20261115` in strict mode on the recording's
  frame schedule through update 900, comparing all three RNGs, the
  scheduler cursor, globals, and every object's `selected` flag, `data`,
  `temps`, `attributes`, `last_result`, `frames`, `person.data`,
  `motives`, `active`, `previous`, and `queue` (377 objects at update 1,
  347 at update 900). Running the comparison with no masks produced
  exactly ten raw difference categories across updates 1–900, all known:
  - **UI global 12**: `2010` at updates 214–393 and `-1` at 394–900 vs
    `0`.
  - **Routing `person.data[37]` (marked unknown in `routing.unknown_words`)**:
    object 16 (`3097` vs `0`, 532 updates across 351–900), object 18
    (`3121` vs `0`, 524 updates across 311–900), object 21 (`3104` vs
    `0`, 85 updates across 233–502), and object 259 (`3287` vs `0`, 887
    updates across 14–900).
  - **Frame locals marked unknown in `frame_storage`**:
    object 21 (Mortimer) frame 8 (0-based), tree `8301`, node `0`, local
    `0` (`6448` vs `0`, updates 675–715); object 259 (salesman) frame 2
    (0-based; 3rd frame), tree `4099` (`0x1003`), node `48`, local `7`
    (`2623` vs `0`, updates 14–596).
  - **Unmarked uninitialized frame-arena locals**:
    object 18 (Bella) frame 6 (0-based), tree `8292`, node `17`, local
    `0` (`16104` vs `0`, updates 203–227), and frame 9 (0-based), tree
    `8229`, node `51`, locals `2` and `3` (`-9320` and `16069` vs `0`,
    updates 368–408; neither is marked by `frame_storage`).
- After centring on Bella at update 543 (`origin = [472, -117]`,
  matching `after-centre.png` to `-1` pixel in x and `0` in y), the
  recording's raw unshifted hover and click points hit the exact same
  objects (`None`, `None`, fridge `118`, then counter `119`, `None`,
  fridge `118`) and open the fridge's pie at the recording's exact label
  rectangles.
- Selected checkpoints of the 900-update replay are pinned by
  `the_recorded_switch_order_replayed_in_the_window_matches_the_recording`
  in `crates/sims-game/tests/goth.rs`, along with
  `centring_a_multi_tile_object_averages_its_parts` (pinning object 25's
  4 parts to `[432, -142]`) and unit tests for
  `fine_point_with_heights`, `tile_corners`, and `scroll_limits`.

## Unverified

- The 1-pixel horizontal offset between `sims-render`'s drawing and
  `Sims.exe` screenshots (`-1` pixel in x both before and after
  centring, consistent with the `-97` opening offset) was not traced or
  changed.
- Hiding the entire UI (`F10`, `0x53b2eb` expanding the house view back
  to full height) and tall Buy/Build strips (`height - 150`) are not
  ported.
