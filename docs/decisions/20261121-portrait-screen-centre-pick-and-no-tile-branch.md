# Dialog portrait screen-centre pick and no-tile fallback (`0x4a7b58`, `0x434891`, `0x43491c`, `0x4e9706`, `0x4e938c`)

*Decided 2026-11-21.*

## Context

`20261120-a-step-runs-both-ways-while-a-refresh-is-pending.md` left two open
items in the dialog portrait path (`0x4e92a8` → `0x4e8c5d` → `0x4e9652`):

1. **Screen-centre pick and search start resolution (`0x4a7b58`, `0x434861`–`0x434921`).**
   Previously, `Lot::set_view_centre` / `Interpreter::view_centre` took a
   pre-resolved lot tile and `sims-game` never passed its camera centre to the
   lot.
2. **No-tile portrait branch (`0x4e9706`, `0x4e9260`, `0x4e938c`).**
   Previously, any start from which `0x4347f0` found no free tile stopped
   loudly — including the four no-room tiles (`19,24; 23,31; 26,28; 26,31`) at
   Goth tick 24,719 (`Waggles`, `0xDC7419A2`, `User00036.iff`), even though
   `User00036.iff` carries `BMP_` 2002.

## Evidence in the original binary

1. **Screen-centre pick (`0x4a677c` → `0x4a7b58` → `0x4a97c3` with flag 1).**
   - `0x4a7b58` computes `px = (rect.right - rect.left) / 2` and
     `py = (rect.bottom - rect.top) / 2` (signed truncating division toward
     zero) and calls `cTSWorldView::vtable[+0x4]` (`0x4a97c3`) with flag `1`.
   - With flag `1` (`0x4a97dc` → `0x4a99f9`), `0x4a97c3` skips the depth
     surface completely and calls `0x4a955d` (coarse screen-to-tile search over
     raised terrain corners on the shown floor, already ported as
     `sims_render::cutaway::View::coarse`) followed by the sub-tile offset
     calculation (`View::fine`), returning a view-rotated fine point
     `[(tx << 4) | fx, (ty << 4) | fy]` with `fx, fy in 0..=15`.
2. **Search start bounds check, fallback, and inverse turn (`0x434861`–`0x434921`).**
   - `0x434885` (`0x433067`) cuts the fine point to a signed-byte view tile
     `[(x >> 4) as i8, (y >> 4) as i8]`.
   - `0x434891` (`0x434cb7`) checks that tile against the rotated lot's
     interior bounds (`[min_x + 1..=max_x - 1, min_y + 1..=max_y - 1]`, set by
     `0x4791a6` from the 64-tile turn table `0x407704`).
   - If outside (`0x43489e`–`0x4348fa`), it replaces the point with the rotated
     midpoint tile `[(size / 2) + min_x, (size / 2) + min_y]` (wrapping byte
     addition via `0x434c85`), converted to fine centre `((t << 4) | 8)`
     (`0x434c98`).
   - `0x4348fd`–`0x43491c` always calls `0x47bc1f(&x, &y, (-rotation) & 3, 0x400)`
     to undo the view's rotation before `0x4349e5` shifts right by 4 and clamps
     to `1..=(size - 2)` (with `.max(1)` last).
   - Because `1023 = (63 << 4) | 15`, `((1023 - ((t << 4) | f)) >> 4) as i8`
     equals `63 - (t as i8)` for every `f in 0..=15` and every `t`, so the
     view-rotated tile pick `[(x >> 4) as i8, (y >> 4) as i8]` determines the
     start tile losslessly.
3. **No-tile branch and `BMP_` 2002 fallback (`0x4e92a8`, `0x4e9652`, `0x4e9706`, `0x4e9260`, `0x4e938c`).**
   - In `0x4e9652`, if `0x4347f0` returns `false`, `0x4e9706` immediately calls
     `0x44123f` (remove object now) on the unput person — skipping virtual
     function `0x88` (`0x45e4e9` / `0x44752e` `placed_from_outside`) — and
     returns `NULL`.
   - Back in `0x4e92a8`: slot 1 (`entry + 0x14`) still has no image, so
     `0x4e9338` calls `0x4e9260(entry, 2002, &bmp)` (`0x5a5752` → `0x4dece6`
     loading `BMP_` 2002 from the person definition's IFF file).
   - If `BMP_` 2002 loads (`0x4e9348`), it stores slot 0 (`0x4e899e`),
     composites with `[0x6f26a8]` (`kPeopleTemplate = 4600` in
     `UIGraphics.far`, loaded at `0x4e89e6`), stores slot 1 (`0x4e937e`,
     clearing slot 1's dirty byte `+0x18`), and does **not** call `0x4e8c5d`
     again (`0x4e938c`).
   - If `BMP_` 2002 is absent (`0x4e9385`), `0x4e938c` calls
     `0x4e8c5d(index, 1, 1)`. Whether that is a **second** call (slot 1's dirty
     byte was set by `0x4eb0eb` / `0x4e895e`) or the **first** call (slot 1's
     dirty byte starts clear at `0x4e88fa`) depends on the dirty byte, which
     `World::portraits` does not track separately.

## Decision

1. **Screen-centre pick (`sims-render`, `sims-vm`, `sims-game`):**
   - Expose `sims_render::screen_centre_pick(lot, camera, size)` calling
     `View::new(lot, camera).fine([size[0] / 2, size[1] / 2])` (`0x4a7b58` →
     `0x4a97c3(1)`).
   - Treat `Interpreter::view_centre` / `Lot::set_view_centre` (`Option<[i8; 2]>`)
     as the view-rotated tile pick (`0x4a7b58` cut via `0x433067`), and resolve
     it in `placement::portrait_start` using `0x434891` bounds check + midpoint
     fallback and `0x43491c` inverse rotation before `search`'s `0x4349e5`
     clamp. Bump `sims_vm::boot::save::FORMAT` from 3 to 4 for the coordinate
     frame change.
   - Wire `sims_game::Game::sync_view` in `open`, `apply`, `resize`,
     `centre_on`, and `command` so every camera move updates both
     `Lot::set_view_rotation` and `Lot::set_view_centre` (while `Game::restore`
     preserves the saved lot's `view_centre`).
2. **No-tile branch (`sims-vm`):**
   - Record `Library::has_portrait_bitmap` in `Program::load` by checking
     whether `BMP_` 2002 exists and decodes cleanly.
   - Split portrait placement into `portrait_search` (`Found`, `NotFound`,
     `Mixed`) and `portrait_place`.
   - On `NotFound`: if `has_portrait_bitmap` is true, remove the person unput
     (`remove_now`, skipping `portrait_place` / `placed_from_outside`) and mark
     `World::portraits` populated (`0x4e9338`–`0x4e937e`). If false, fail
     loudly because `0x4e938c`'s build count depends on slot 1's untracked
     dirty flag.
   - On `Mixed` (`view_centre == None`, some starts find a tile and others find
     none): if `has_portrait_bitmap` is true, run both the placed branch and
     the unput branch from the post-search state and keep the result only if
     both leave bit-for-bit identical worlds and cells; otherwise fail loudly.

## Results

At Goth tick 24,719 (`Waggles`, `User00036.iff`, which has `BMP_` 2002), all
3,844 start tiles — including the four no-room cells (`19,24; 23,31; 26,28;
26,31`) — and `view_centre = None` now succeed and reach the exact same state
hash (`every_start_gives_one_state` in `crates/sims-vm/tests/boot.rs`).
All existing Goth, Newbie, Old Town, and UD2 snapshot pins remain unchanged.
