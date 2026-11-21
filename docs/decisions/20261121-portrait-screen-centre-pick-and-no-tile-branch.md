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
     `-2 * ARRAY_SIZE..3 * ARRAY_SIZE`: `0x4a9581`–`0x4a9588` narrows the
     initial screen deltas to signed 16-bit values, and `0x4a9c5c`–`0x4a9c71`
     / `0x4b3c90`–`0x4b3c93` / `0x4a9d9e`–`0x4a9db6` narrow each projected tile
     to signed bytes and return signed 16-bit screen coordinates. `0x4a9a29`–`0x4a9b4d`
     applies the same signed-byte tile narrowing and 16-bit screen coordinate
     arithmetic in `View::fine`. For example, at far zoom on an 800×600 view,
     origin `[400, -2900]` returns coarse `[200, 200]` (signed byte
     `[-56, -56]`, triggering the rotated midpoint fallback) and origin
     `[381, -3997]` returns fine `[4306, 4303]` (coarse `[269, 268]`, signed
     byte `[13, 12]`, passing the interior bounds check).
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
     (`0x4538fd`–`0x4539cf` → `0x4539c7` reading `[esi+0x11c]` → `0x45810c(1)`
     → `0x4eb0eb` → `0x4e895e`, using the **running** object's definition even
     for scope-19 `Stack Person Data` writes) or creating a person with
     `pd[84]&1 || pd[68]&1 || pd[74]&3` (`0x44e6b6`–`0x44e712`) releases all
     seven cached images and sets every slot's dirty byte `+0x18` to `1`.
     Storing any non-null image (`0x4e899e`) clears that slot's dirty byte to
     `0`.
   - **Clean, empty slot with `BMP_` 2002 (0 builds):** at `0x4e92e5`,
     `slot1.dirty == 0` skips `0x4e92f2` (`0x4e8c5d(index, 1, 0)`). Slot 1 is
     still empty (`0x4e9315`), so `0x4e9338` calls `0x4e9260(entry, 2002, &bmp)`,
     stores slot 0 (`0x4e9354`), composites with `[0x6f26a8]` (`kPeopleTemplate
     = 4600`), stores slot 1 (`0x4e937e`, clearing dirty), and returns `true`
     without ever creating a temporary person, running `Init`, drawing RNG, or
     checking household membership.
   - **Read-only resources (`0x4e8cc6`):** inside `0x4e8c5d`, an off-lot build
     checks `iResFile::CanWrite` (`vtable[+0x20]`) before calling `0x4e9652`.
     Only loose `.iff` files under `UserData*/Characters` (`0x43c6b9`,
     `0x43c2c1`, `0x43c5bf` → `0x43e48a` → `0x465970` → `0x413083`) whose
     read-write open succeeds (`0x413094`–`0x4130dd`, falling back to read-only
     when open fails) and runtime-made character files are writable; FAR
     archive members (`ResolveResFile`, `0x5cfb38`) and non-`Characters` mounts
     (`0x43aa03`) return `false`. For a read-only off-lot resource, `0x4e8c5d`
     skips `0x4e9652` completely, so `0x4e9338` loads `BMP_` 2002 (clearing
     dirty) when present and otherwise leaves slot 1 empty with zero builds.
   - **Dirty, empty slot on a writable resource with `BMP_` 2002 (1 build +
     no-tile fallback):** at `0x4e92e5`, `slot1.dirty != 0` calls
     `0x4e92f2: 0x4e8c5d(index, 1, 0)` (`save = 0`). If `0x4347f0` finds a
     tile, `0x4e96fe` places the person (virtual `0x88`), draws it, stores slot
     1 (clearing dirty, without saving `BMP_` 2002), and removes it
     (`0x4e8fdb`). If no tile is found (`0x4e9706`), it removes the person
     unput (`0x44123f`, skipping virtual `0x88`) and returns `NULL`; back in
     `0x4e92a8`, slot 1 is still empty, so `0x4e9338` loads `BMP_` 2002, stores
     slot 1 (clearing dirty), and skips `0x4e938c` (no second build). At Goth
     tick 24,716, Waggles (`0xDC7419A2`) runs `CatGlobals.iff` BHAV 8328
     (`Kill pet for Good`) node 16 (`My Person Data 68 := 1`), marking slot 1
     dirty via `0x4539cf` → `0x4eb0eb`; three ticks later at 24,719,
     `UrnStonePet.iff` shows the death dialog on that dirty slot, triggering
     the single 1,920-draw build recorded in `goth-day-20261115`.
   - **Missing `BMP_` 2002 and saving on first build (`0x4e9385` → `0x4e938c` →
     `0x4e8d2e` → `0x4e9000`):** when `0x4e9260(2002)` fails, `0x4e938c` calls
     `0x4e8c5d(index, 1, 1)` (`save = 1`). When that build succeeds (on-lot or
     writable off-lot `Found`) on a writable file (`0x4e9031`), `0x4e9000`
     writes `BMP_` 2002 into the type's resource file (`0x4e904e`–`0x4e90ab`),
     so any subsequent lookup (`0x4e926a`–`0x4e9287`) for any definition in that
     same resource file sees `BMP_` 2002. On a writable off-lot `NotFound`, a
     dirty slot would run a second build at `0x4e938c` and a clean slot leaves
     slot 1 empty; both fail loudly.
   - **Runtime-made person types (`0x4265fd` → `0x4e8a3a` → `0x4e8add`):**
     cloning a character copies every resource currently in the template's file
     (`0x472356`–`0x472519`, including load-time `BMP_` 2002 on shipped
     `TemplateCat.iff` / `TemplateDog.iff` as well as any `BMP_` 2002 saved
     into a writable template during play) into a new writable `User%05d.iff`
     and calls `0x4e8a3a`, which appends every new `type == 2` definition to
     the portrait table `0x6f2698` once initialized (`[0x6f26a4] = 1` at
     `0x4e8a32`). Positive-age pet creation in `Make New Character`
     unconditionally calls `0x44b126` → `0x4e8c5d(index, 0x1f, 1)` regardless
     of whether the cloned template already carried `BMP_` 2002, creating a
     temporary pet, running `Init`, placing and saving `BMP_` 2002 + populating
     slot 1 on `Found`, or removing unput (`0x4e9706`) and returning `false`
     (ignored by `0x44b126`) on `NotFound`.

## Decision

1. **Screen-centre pick (`sims-render`, `sims-vm`, `sims-game`):**
   - Expose `sims_render::screen_centre_pick(lot, camera, size)` calling
     `View::new(lot, camera).fine([size[0] / 2, size[1] / 2])` (`0x4a7b58` →
     `0x4a97c3(1)`). Remove the `-2 * ARRAY_SIZE..3 * ARRAY_SIZE` bailout in
     `View::coarse`, match `0x4a9581`–`0x4a9588` signed 16-bit screen deltas
     and `0x4a9c5c`–`0x4a9c71` / `0x4b3c90`–`0x4b3c93` signed-byte tile
     projection narrowing, and fast-forward negative off-lot scans in O(1) so
     off-lot cameras match `0x4a955d` without panicking on extreme coordinates.
   - Treat `Interpreter::view_centre` / `Lot::set_view_centre` (`Option<[i8; 2]>`)
     as the view-rotated tile pick (`0x4a7b58` cut via `0x433067`), and resolve
     it in `placement::portrait_start` using `0x434891` bounds check + midpoint
     fallback and `0x43491c` inverse rotation before `search`'s `0x4349e5`
     clamp. Retain `Lot::replay_view` across `Stop::Asks`, `save`, and
     `restore` so moving the camera while a dialog waits does not alter the
     tick's replay once answered. Bump `sims_vm::boot::save::FORMAT` from 4
     (keeping `floor_rotation`) to 5.
   - Wire `sims_game::Game::sync_view_centre` / `Game::sync_view` (passing
     `self.house_viewport_size()`, reflecting the active expanded vs collapsed
     control-panel strip, and the native origin column `camera.origin[0] +
     pick::ORIGIN_COLUMN`) in `open`, `resize`, `centre_on` (which also uses
     `self.house_viewport_size()`), `command` (including `Command::TogglePanel`),
     and in `apply` (anchoring zoom and rotation via `native_zoom_or_rotate`
     using `0x4a7b58` on the displayed floor, `0x434cb7` + `(size / 4) + min`
     fallback, `0x47bc1f` about `1023`, and `0x4a7996`/`0x4a7af0`/`0x4a81cc`
     signed-byte-narrowed terrain-aware recentering, syncing only when camera
     geometry changes so no-op or wall-mode commands preserve a restored pick).
2. **Dirty-slot gate, writability, saved bitmaps, and no-tile branch (`sims-sim`, `sims-vm`):**
   - Append `World::dirty_portraits: BTreeSet<u32>` and
     `World::saved_portraits: BTreeSet<u32>` (bumping `sims_sim::SNAPSHOT_FORMAT`
     from 31 to 32, with format 31 as an exact prefix). Mark dirty and clear
     `portraits` on person data writes to words 68, 74, and 84
     (`0x4538fd`–`0x4539cf`, on `[esi+0x11c]`, the running object's definition)
     and on `make_person` when `pd[84]&1 || pd[68]&1 || pd[74]&3`
     (`0x44e712`); clear dirty when `portraits` is populated (`0x4e899e`).
   - Record `Library::has_portrait_bitmap` (`BMP_` 2002 exists and decodes) and
     `Library::writable` (loose file under the `Characters` mount whose
     read-write open succeeds, `0x413094`–`0x4130dd`) in `Program::load`; treat
     runtime-made person types (`World::characters`) as listed (`0x4265fd` →
     `0x4e8a3a`), writable, and inheriting both load-time `has_portrait_bitmap`
     and any template `BMP_` 2002 saved before cloning (`0x472356`–`0x472519`).
     Run `0x44b126` → `0x4e8c5d(index, 0x1f, 1)` directly in `make_character`
     for positive-age pets. Look up `World::saved_portraits` at the
     resource-file level for non-made catalog definitions (`0x4e904e`–`0x4e90ab`,
     `0x4e926a`–`0x4e9287`) and per-GUID for runtime-made characters.
   - Split portrait placement into `portrait_search` (`Found`, `NotFound`,
     `Mixed`) and `portrait_place(&World, ...)` (borrowing `&World` immutably
     and touching no interpreter cells, explicitly rejecting `object.selected`,
     `pd[77] > 0`, and `pd[81] >= 10`; native put+remove equivalence for rooms
     and tiles remains an unverified assumption as documented in
     `placement.rs`).
   - In `dialog_portrait`:
     - `on_lot`: record `saved_portraits` if `!dirty && !has_bitmap && writable`,
       insert into `portraits`, clear `dirty_portraits`.
     - `!on_lot`:
       - Clean + `has_bitmap`: insert into `portraits` with **0 builds** (even
         for household members).
       - `!writable`: **0 builds** (`0x4e8cc6`); if `has_bitmap`, insert into
         `portraits` and clear `dirty_portraits`; otherwise leave slot 1 empty.
       - Writable + (`dirty || !has_bitmap`): check household guard, then run
         1 build; on `Found` record `saved_portraits` only when `!dirty`
         (`save = 1`); on `NotFound` remove unput (`0x4e9706`) when `has_bitmap`
         or fail loudly otherwise; on `Mixed` check `portrait_place` when
         `has_bitmap` or fail loudly otherwise.

## Results

At Goth tick 24,719 (`Waggles`, `User00036.iff`, dirtied at tick 24,716 by
`Kill pet for Good` writing `pd[68] := 1` and carrying `BMP_` 2002), all 3,844
start tiles — including the four no-room cells (`19,24; 23,31; 26,28; 26,31`)
— and `view_centre = None` succeed with the recorded 1,920 CRT draws and reach
the exact same state hash (`every_start_gives_one_state` in
`crates/sims-vm/tests/boot.rs`). Clean slots with `BMP_` 2002 populate
directly with zero temporary-person builds. All format-31 prefix pins of the
Goth, Newbie, Old Town, and UD2 tests are preserved under snapshot format 32.
