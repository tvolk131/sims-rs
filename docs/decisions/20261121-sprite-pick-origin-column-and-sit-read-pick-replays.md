# Sprite pick and centring use the original's origin column; Sit and Read pick captures settle both open pointer mysteries

*Decided 2026-11-21.*

Replaying Track 2's two newly completed 900-update Goth diagnostic captures
(`goth-read-pick-20261121` and `goth-sit-shots-20261121`) through `sims-game`'s
window input layer, together with `Sims.exe` disassembly and the memory dumps
of `goth-read-pick-20261121`, settles both open questions from
`20261119-pointer-pick-is-the-originals-sprite-pick.md` and
`20261120-read-order-replayed-in-the-window.md`, as well as the 1-pixel
horizontal offset noted in
`20261121-switch-order-replayed-and-centre-view-terrain.md`.

## Settled: why the recorded bookcase point `(384, 250)` picked the door

- **What `Sims.exe` does (`0x4b3c88`, `0x4ac14b`, `0x4ac25d`, `0x4a97c3`, `0x438f71`).**
  `0x4b3c88` projects tile corners using `DAT_006f00d0` (`origin_x`), while
  `0x4ac14b` places every straight wall (`1`, `2`) and across-diagonal wall
  (`0x10`) one column left of that corner (`*param_2 = -1 - (8 << zoom)` or
  `-1`), `0x53e89f` shifts edge-on diagonals left by one (`DEC EAX`), and
  `0x4ac25d` similarly shifts tile rects by `-1`. Objects, people, depth
  unprojection (`0x4a97c3`), and drawn-rectangle widening (`0x438f71`) all
  use `origin_x` directly.
- **Our convention (`OBJECT_COLUMN = 1`).** `sims-render` places walls and
  floors at `camera.origin[0]` and shifts objects (`OBJECT_COLUMN = 1`),
  people (`sims-people`'s `OBJECT_COLUMN = 1.0`), grass (`GRASS_COLUMN = 1`),
  and corner caps (`cx + 1`) one column right
  (`20261114-objects-sit-one-pixel-right-walls.md`). That means our
  `camera.origin[0]` equals `origin_x - 1`, and the original's draw-context
  origin column is `camera.origin[0] + 1`.
- **The bug in `pick::sprite` and `Game::centre_screen_point`.**
  - In `dump-0095` (`goth-read-pick-20261121`, `origin = (368, -130)`), pixel
    `(384, 250)` sits on column `1` of the standing wall along `x = 26` with
    depth `45117`. Both `Sims.exe`'s inverse view matrix (`0x4a97c3`) and
    `Depth::unproject` unproject `(384 - 368 = 16, 250, 45117)` to fine
    `(415.496, 399.496)` → rounded `(415, 399)` → tile `(25, 24)`, which is
    bookcase `132`'s own tile (`+0x160 = [4, 323, 32, 381]`).
  - Previously, `pick::sprite` passed canvas `x` directly to `Depth::unproject`
    and `camera.unproject_fine` (subtracting `camera.origin[0]` = `464`
    instead of `464 + 1 = 465`), inflating `x - origin_x` by `+1` pixel
    (`17` instead of `16`) and shifting `fine_x` by `+0.5` to `415.996` →
    rounded `416` → tile `(26, 24)` in front of the bookcase, so the forward
    walk skipped `(25, 24)` and hit door `51`.
  - Similarly, `pick::widen` (`0x438f71`) previously rounded screen `left` and
    `right` to multiples of 4 in raw canvas coordinates rather than in
    lot-relative coordinates (`x - origin_x`).
  - Finally, `Game::centre_screen_point` (`0x4a81cc`) stored the aligned
    native `origin_x` directly into `camera.origin[0]` (leaving walls and
    objects 1 pixel right of `Sims.exe`, e.g. `[472, -117]` instead of
    `[471, -117]` after centring on Bella in `goth-switch-cancel-order-20261115`,
    where window `80` then covered `x = 280` instead of ending at `280`).
- **Ported.**
  - `pick::ORIGIN_COLUMN = 1` (`crates/sims-game/src/pick.rs`): `pick::sprite`
    passes `[x - 1, y]` to `place` (`Depth::unproject`) and
    `camera.unproject_fine`, and widens `Canvas::drawn` rectangles to
    4-column multiples relative to `origin_x = camera.origin[0] + 1`
    (leaving empty rectangles `left >= right` untouched, matching `0x438f71`).
  - `Game::centre_screen_point` (`crates/sims-game/src/game.rs`): computes
    `native_x = clamped.wrapping_add(1) & !3` (`0x4a81cc`) and stores
    `native_x - pick::ORIGIN_COLUMN`, landing on `[471, -117]` for Bella at
    update 543 (0-pixel shift in both x and y against `after-centre.png`,
    which correlates at `dx = +104, dy = +13` from `before-read.png`, and
    keeping `game.under([280, 235]) == None` at update 603).
  - `the_recorded_read_order_replayed_in_the_window_matches_the_recording`
    now pins `game.under(recorded([384, 250])) == Some(132)` directly, with
    the 7-column workaround removed.
  - **Follow-ups outside this change:** `sound::screen` (`0x570e09`,
    `crates/sims-game/src/sound.rs`) and `Cutaway::coarse` (`0x4ab7ee`,
    `crates/sims-render/src/cutaway.rs`) still project/unproject directly
    against `camera.origin[0]` (the tile-art origin) rather than
    `camera.origin[0] + 1`; aligning those two consumers is left for
    dedicated follow-up changes so their public projection APIs and
    `sims-render` stay untouched here.

## Settled: "under the still pointer" listings are a Wine 8.0 `GetClipStatus` stub artifact

- Both `goth-read-pick-20261121` and `goth-sit-shots-20261121` watch the
  noted-3D global `0x6f6bc4` (`pick` / `pick85` / `pick720`) and the house
  tool's current-object word (`tool` / `tool85` / `tool720`), and
  `goth-read-pick-20261121` additionally watches object `+0x170` (`r18a/b`,
  `r383a/b`) and dumps process memory at updates 85, 90, 95, 720, 725, 730.
- `0x6f6bc4` stayed `0` across every drawn frame in
  `goth-sit-shots-20261121` (`pick`), and across both watched diagnostic
  windows (`85–100` `pick85` and `720–730` `pick720`, confirming zero
  `0x6f6bc4` at updates 93 and 723 while `frame1`/`frame2`/`frame3` covered
  `1–84`, `101–719`, and `731–900`) in `goth-read-pick-20261121`. Every
  still-pointer listing on empty lawn or distant ground came from the 2D
  sprite pick (`0x4aa713`) testing `+0x170`:
  - In Wine 8.0 (`Debian 8.0~repack-4`), `IDirect3DDevice3::GetClipStatus`
    is a stub returning the entire viewport `(0, 0, 800, 600)`.
  - `0x43776a` subtracts the draw-context origin `ctx(+0x3c, +0x40) = (368, -130)`
    via `0x4b4470` and writes `+0x170 = (-368, 130, 432, 730)` — the whole
    800×600 screen — onto every drawn 3D object.
  - Memory dumps `dump-0095` and `dump-0725` confirm `+0x170 = [-368, 130, 432, 730]`
    on every on-screen 3D object: Alexander (`16`), Bella (`18`), Mortimer
    (`21`), the plumb bob (`383`), and (once on screen at `dump-0725`, tile
    `(31, 29)`) salesman `259`.
  - Consequently, whenever a 3D Sim or plumb bob steps onto any of the 21
    tiles walked forward by `0x4aa713`, its full-screen `+0x170` contains the
    still pointer:
    - `goth-read-pick-20261121`, update 723: `dump-0720`, `dump-0725`, and
      `dump-0730` give depth `42452` at `(384, 324)`, which the native
      inverse matrix places on floor 1 at tile `(30, 29)`; step 1 reaches
      salesman `259`'s tile `(31, 29)`, writing `tool720: 0 -> 259` while
      `pick720` stays `0 -> 0`.
    - `goth-sit-shots-20261121`, updates 275 and 305: from `(432, 155)`
      (tile `(21, 17)` on the plateau), the walk hits Alexander `16` at
      `(26, 21)` (`tool: 0 -> 16`) and then Mortimer's plumb bob `383` at
      `(27, 22)` (`tool: 16 -> 383`) while `pick` stays `0`.
- We intentionally do **not** replicate Wine's stubbed `GetClipStatus`
  full-screen `+0x170` in `sims-game`: genuine Direct3D 3 `GetClipStatus`
  bounds a 3D mesh tightly rather than to the whole viewport, and our pick
  correctly returns `None` on those empty-lawn pixels.

## Verified

An offline unmasked comparison (scratch tools, not in the repository)
replayed both 900-update captures in strict mode on their exact watched frame
schedules, comparing all three RNGs (`lot_rng`, `global_rng`, `crt_rng`),
`tick`, `action_serial`, `scheduler_cursor`, `paused`, `mode`,
`budget_balance`, all 64 globals, and every object's ID set (377 at update 1;
346 at update 900), `selected`, `position`, `registry_word`, `data`, `temps`,
`attributes`, `last_result`, `frames`, `person.data`, `motives`,
`previous_motives`, `room`, `last_decay`, `active`, `previous`, and `queue`.

1. **`goth-read-pick-20261121`** (`0,1,2,3,33,63,93,123,145,165,183+30`) —
   exactly 8 raw differences across updates 1–900:
   - **Routing `person.data[37]` (all marked unknown in `routing.unknown_words`):**
     object 16 (`0` vs `3096`, 477 updates across 351–647 and 721–900),
     object 18 (`0` vs `3100`, 171 updates 311–481),
     object 21 (`0` vs `3109`, 85 updates across 184–197, 368–402, 418–453),
     object 259 (`0` vs `3107`, 887 updates 14–900).
   - **Stale frame locals (all marked unknown in `frame_storage`):**
     object 21 frame 9 (0-based), tree `8301`, local `0` (`0` vs `6448`,
     updates 668–678);
     object 259 frame 2 (0-based), tree `4099`, local `7` (`0` vs `2623`,
     updates 14–596).
   - **Wine `GetClipStatus` still-pointer listing of salesman 259 at 723:**
     object 21 `data[50]` (`0` vs `1`, updates 724–900) and `last_result`
     (`0` vs `1`, updates 724–868).

2. **`goth-sit-shots-20261121`**
   (`0,1,2,3,33,63,93,123,153,171,192,215,245,275,305,335,365,395,425,455,485,515,545,575,605,635,665,695,725,754,773,788+30`)
   — both Sit (bench 173, active at 216) and Read a Book (bookcase 132,
   hovered directly at `(384, 250)` at 695, queued behind Mortimer's active
   action at 789, active by 900) succeed, with exactly 8 raw differences
   across updates 1–900:
   - **Routing `person.data[37]` (all marked unknown in `routing.unknown_words`):**
     object 16 (`0` vs `3004`, 166 updates 735–900),
     object 18 (`0` vs `3083`, 122 updates across 311–366 and 538–603),
     object 21 (`0` vs `3003`, 58 updates 843–900),
     object 259 (`0` vs `3287`, 583 updates 14–596).
   - **Stale frame locals (all marked unknown in `frame_storage`):**
     object 21 frame 5 (0-based), tree `8228`, local `2` (`0` vs `55440`,
     updates 478–498);
     object 259 frame 2 (0-based), tree `4099`, local `7` (`0` vs `2623`,
     updates 14–596).
   - **Wine `GetClipStatus` still-pointer listings (Alexander 16 at 275,
     plumb bob 383 at 305):**
     object 21 `last_result` (`1` vs `0`, updates 276–305) and `data[50]`
     (`0` vs `1`, updates 306–695, cleared back to `0` at 696 when hovering
     bookcase 132 at 695).

Selected checkpoints of both replays are pinned by
`the_recorded_read_order_with_the_pick_watched_replayed_in_the_window_matches_the_recording`
and
`the_recorded_sit_and_read_orders_with_shots_replayed_in_the_window_matches_the_recording`
in `crates/sims-game/tests/goth.rs`, alongside unit tests for origin-relative
widening in `crates/sims-game/src/pick.rs`.

## Unverified

- Genuine Windows Direct3D 3 `GetClipStatus` 3D screen-rectangle bounds for
  people and the plumb bob (which are tight around the rendered mesh rather
  than full-screen) are not modeled in `Canvas::drawn`; our 3D picks rely on
  exact rendered pixels (`pick::noted`) alone.
