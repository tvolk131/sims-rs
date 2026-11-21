# Dialog portrait screen-centre pick, dirty-slot gate, and no-tile fallback (`0x4a7b58`, `0x434891`, `0x43491c`, `0x4e92e5`, `0x4e9706`, `0x4e938c`)

*Decided 2026-11-21.*

## Context

`20261120-a-step-runs-both-ways-while-a-refresh-is-pending.md` left two open
items in the dialog portrait path (`0x4e92a8` → `0x4e8c5d` → `0x4e9652`):

1. **Screen-centre pick and search start resolution (`0x4a7b58`, `0x434861`–`0x434921`).**
   Previously, `Lot::set_view_centre` / `Interpreter::view_centre` took a
   pre-resolved lot tile and `sims-game` never passed its camera centre to the
   lot.
2. **Dirty-slot gate and no-tile portrait branch (`0x4e92e5`, `0x4e9706`, `0x4e9260`, `0x4e938c`).**
   Previously, any start from which `0x4347f0` found no free tile stopped
   loudly — including the four no-room tiles (`19,24; 23,31; 26,28; 26,31`) at
   Goth tick 24,719 (`Waggles`, `0xDC7419A2`, `User00036.iff`), even though
   `User00036.iff` carries `BMP_` 2002 and `Waggles` was marked dirty at tick
   24,716 (`CatGlobals.iff` `Kill pet for Good` writing `pd[68] := 1`).

## Evidence in the original binary

1. **Screen-centre pick (`0x4a677c` → `0x4a7b58` → `0x4a97c3` with flag 1).**
   - `0x4a7b58` computes `px = (rect.right - rect.left) / 2` and
     `py = (rect.bottom - rect.top) / 2` (signed truncating division toward
     zero) over `cTSWorldView`'s rect above the 100-pixel live-mode strip
     (`0x49ae5e` → `0x5cf1b6`: `(width, height - 100)`) and calls
     `cTSWorldView::vtable[+0x4]` (`0x4a97c3`) with flag `1`.
   - With flag `1` (`0x4a97dc` → `0x4a99f9`), `0x4a97c3` skips the depth
     surface completely and calls `0x4a955d` (coarse screen-to-tile search over
     raised terrain corners on the shown floor, ported as
     `sims_render::cutaway::View::coarse`) followed by the sub-tile offset
     calculation (`View::fine`), returning a view-rotated fine point
     `[(tx << 4) | fx, (ty << 4) | fy]` with `fx, fy in 0..=15`.
   - `0x4a955d` does not bail out when the initial guess lies outside
     `-2 * ARRAY_SIZE..3 * ARRAY_SIZE`: e.g., at far zoom on an 800×600 view
     with origin `[400, -2900]`, it returns `[200, 200]`, which `0x433067`
     cuts to signed byte `[-56, -56]`, sending `0x434891` to the rotated
     midpoint fallback rather than treating the camera pick as unknown.
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
3. **Dirty-slot gate, clean-slot `BMP_` 2002 path, and dirty-slot no-tile fallback (`0x4e92a8`, `0x4e92e5`, `0x4e9652`, `0x4e9706`, `0x4e9260`, `0x4e938c`, `0x4538fd`–`0x4539cf`).**
   - Every portrait table entry starts with all seven slots empty and clean
     (`0x4e890a` → `0x4e88fa`). Writing person data words 68, 74, or 84
     (`0x4538fd`–`0x4539cf` → `0x45810c(1)` → `0x4eb0eb` → `0x4e895e`) or
     creating a person with `pd[84]&1 || pd[68]&1 || pd[74]&3` (`0x44e6b6`–`0x44e712`)
     releases all seven cached images and sets every slot's dirty byte `+0x18`
     to `1`. Storing any non-null image (`0x4e899e`) clears that slot's dirty
     byte to `0`.
   - **Clean, empty slot with `BMP_` 2002 (0 builds):** at `0x4e92e5`,
     `slot1.dirty == 0` skips `0x4e92f2` (`0x4e8c5d(index, 1, 0)`). Slot 1 is
     still empty (`0x4e9315`), so `0x4e9338` calls `0x4e9260(entry, 2002, &bmp)`,
     stores slot 0 (`0x4e9354`), composites with `[0x6f26a8]` (`kPeopleTemplate
     = 4600`), stores slot 1 (`0x4e937e`, clearing dirty), and returns `true`
     without ever creating a temporary person, running `Init`, or drawing RNG.
   - **Dirty, empty slot with `BMP_` 2002 (1 build + no-tile fallback):** at
     `0x4e92e5`, `slot1.dirty != 0` calls `0x4e92f2: 0x4e8c5d(index, 1, 0)`.
     If `0x4347f0` finds a tile, `0x4e96fe` places the person (virtual `0x88`),
     draws it, stores slot 1 (clearing dirty), and removes it (`0x4e8fdb`). If
     no tile is found (`0x4e9706`), it removes the person unput (`0x44123f`,
     skipping virtual `0x88`) and returns `NULL`; back in `0x4e92a8`, slot 1 is
     still empty, so `0x4e9338` loads `BMP_` 2002, stores slot 1 (clearing
     dirty), and skips `0x4e938c` (no second build). At Goth tick 24,716,
     Waggles (`0xDC7419A2`) runs `CatGlobals.iff` BHAV 8328 (`Kill pet for
     Good`) node 16 (`My Person Data 68 := 1`), marking slot 1 dirty via
     `0x4539cf` → `0x4eb0eb`; three ticks later at 24,719, `UrnStonePet.iff`
     shows the death dialog on that dirty slot, triggering the single
     1,920-draw build recorded in `goth-day-20261115`.
   - **Missing `BMP_` 2002 (`0x4e9385` → `0x4e938c`):** `0x4e938c` calls
     `0x4e8c5d(index, 1, 1)`. On `NotFound`, a dirty slot would run a second
     build, and a clean slot leaves slot 1 empty and depends on file
     writability (`0x4e8cc6`, unmodeled); both fail loudly.

## Decision

1. **Screen-centre pick (`sims-render`, `sims-vm`, `sims-game`):**
   - Expose `sims_render::screen_centre_pick(lot, camera, size)` calling
     `View::new(lot, camera).fine([size[0] / 2, size[1] / 2])` (`0x4a7b58` →
     `0x4a97c3(1)`). Remove the `-2 * ARRAY_SIZE..3 * ARRAY_SIZE` bailout in
     `View::coarse`, using `i64` projection bounds and an O(1) fast-forward for
     negative off-lot scans so off-lot cameras return their exact tile pick
     (falling back to the rotated midpoint in `portrait_start`) without
     panicking on extreme coordinates.
   - Treat `Interpreter::view_centre` / `Lot::set_view_centre` (`Option<[i8; 2]>`)
     as the view-rotated tile pick (`0x4a7b58` cut via `0x433067`), and resolve
     it in `placement::portrait_start` using `0x434891` bounds check + midpoint
     fallback and `0x43491c` inverse rotation before `search`'s `0x4349e5`
     clamp. Retain `Lot::replay_view` across `Stop::Asks`, `save`, and
     `restore` so moving the camera while a dialog waits does not alter the
     tick's replay once answered. Bump `sims_vm::boot::save::FORMAT` from 3 to
     4.
   - Wire `sims_game::Game::sync_view` (passing `self.house_view_size()`,
     matching the house view above the 100-pixel strip) in `open`, `resize`,
     `centre_on`, `command`, and in `apply` only when camera geometry
     (`origin`, `zoom`, `rotation`, or `level`) changes so no-op or wall-mode
     commands preserve a restored pick.
2. **Dirty-slot gate and no-tile branch (`sims-sim`, `sims-vm`):**
   - Append `World::dirty_portraits: BTreeSet<u32>` (bumping
     `sims_sim::SNAPSHOT_FORMAT` from 31 to 32, with format 31 as an exact
     prefix). Mark dirty and clear `portraits` on person data writes to words
     68, 74, and 84 (`0x4538fd`–`0x4539cf`) and on `make_person` when
     `pd[84]&1 || pd[68]&1 || pd[74]&3` (`0x44e712`); clear dirty when
     `portraits` is populated (`0x4e899e`).
   - Record `Library::has_portrait_bitmap` in `Program::load` by checking
     whether `BMP_` 2002 exists and decodes cleanly.
   - Split portrait placement into `portrait_search` (`Found`, `NotFound`,
     `Mixed`) and `portrait_place(&World, ...)` (immutable borrow, explicitly
     rejecting `object.selected`, `pd[77] > 0`, and `pd[81] >= 10` so `Ok(())`
     statically proves placing leaves the exact same world and cells as
     removing unput).
   - In `dialog_portrait` when `!on_lot`:
     - Clean + `BMP_` 2002: insert into `World::portraits` with **0 builds**.
     - Dirty + `BMP_` 2002: run 1 build; on `NotFound` remove unput (`0x4e9706`,
       skipping `portrait_place`), on `Mixed` verify `portrait_place` before
       removing, and populate `World::portraits` / clear `dirty_portraits`.
     - Missing `BMP_` 2002: run 1 build; on `NotFound` or `Mixed`, fail loudly.

## Results

At Goth tick 24,719 (`Waggles`, `User00036.iff`, dirtied at tick 24,716 by
`Kill pet for Good` writing `pd[68] := 1` and carrying `BMP_` 2002), all 3,844
start tiles — including the four no-room cells (`19,24; 23,31; 26,28; 26,31`)
— and `view_centre = None` succeed with the recorded 1,920 CRT draws and reach
the exact same state hash (`every_start_gives_one_state` in
`crates/sims-vm/tests/boot.rs`). Clean slots with `BMP_` 2002 populate
directly with zero temporary-person builds. All format-31 prefix pins of the
Goth, Newbie, Old Town, and UD2 tests are preserved under snapshot format 32.
