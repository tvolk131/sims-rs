# Corner counter `117` in `near-r3-down-0816` wakes during paused partial passes (`0x46d553`)

*Decided 2026-11-21.*

`near-r3-down-0816` was the only ratchet scene below 99% (`frame` 98.40%, `objects` 94.50%), driven almost entirely by kitchen corner counter ID `117` (`0x91767a36`, `Counter - Kitchen - Moderate` at tile `(16, 30)` in `House05.iff`) drawing straight `DGRP #100` (`SPR2 #100`) in our render while the frozen reference draws corner `DGRP #101` (`SPR2 #111`) — and the same counter's northwest backsplash appeared through the kitchen window in `near-r0-up-0816` (`187` diff pixels). Yet in `far-r0-cut-0816`, `far-r1-cut-0816`, and `medium-r1-cut-0816`, captured from the same paused `Sims.exe` process at the same clock (`08:16:18`, tick `3189`), counter `117` drew straight `DGRP #100`.

This resolves the open note in [`20261121-road-dashes-swap-on-odd-rotation.md`](20261121-road-dashes-swap-on-odd-rotation.md).

## What the original does

Read from the Complete Collection 1.91 executable (`Sims.exe`), `GameData/Objects/Objects.far` (`Counters.iff`), `GameData/Global/Global.far` (`CounterGlobals.iff`), `ExpansionShared/ExpansionShared.far` (`NeighborhoodHelpSystem.iff`), and live Complete Collection captures:

1. **Counter `117` genuinely resets on load (`0x43a516`, `0x431e2e`, `0x431fbe`, `0x431be1`).**
   - `House05.iff` saves counter `117` with `data[0] = 1` (`DGRP #101`), but `Counters.iff` / `CounterGlobals.iff`'s linked `Init` / `Main` signatures in Complete Collection differ from the base-game save.
   - `Sims.exe` flags the type as changed (`0x43a516`), resets all four kitchen counters (`114`–`117`) in `cXObject::PostLoad` → `Reset` (`0x431e2e` → `0x431fbe`), runs `Init` (`BHAV 8223`), and starts `Main` (`BHAV 4096` → `8224` → `Idle(20000)`).
   - `Init` leaves `data[0] = 0` (`DGRP #100`) and sets `data[60] = 1` (`BHAV 8223` node 7: "simulate when paused", which sets scheduler flag bit 17 `0x20000`).
   - Verified across all four live Complete Collection recordings (`goth-day-20261115`, `goth-day-frame-20261121`, `goth-buy-move-sell-20261121`, `goth-build-wall-floor-20261121`): at `entry-0000`, counter `117` is already reset with `data[0] = 0`, `data[60] = 1`, `registry_word = 0x3ffff`, and stack `[(4096, 0), (8224, 3), (280, 0, [20000])]`; `goth-day-20261115`'s `update-3190` snapshot confirms it is still sleeping in `Idle(20000)` at tick `3190` with `data[0] = 0`, `data[60] = 1`, and `registry_word = 0x341aa` (`0x20000 | 0x10000 | 16810`).
2. **Draw selection depends strictly on `data[0]` (`0x4378bc`, `0x45764f`).**
   - `cXObject::Draw` reads `word ptr [edi + 0x4a]` (`data[0]`) at `0x437917` and passes it straight to `cTSObjectDefinition::GetDGRP` (`0x45764f`). `DGRP #101` (`SPR2 #111`) is drawn only when `data[0] == 1`.
3. **Paused partial scheduler passes still count down `Sleep` and run `Simulate()` for `data[60] != 0` objects (`0x46d345` → `0x46d553`).**
   - When the game is paused (`global[17] != 0`, `[esi + 0x32]` at `0x46d560`), `cSimulator::StepObjects` (`0x46d553`) sets its partial-pass flag and returns `0` so `cSimulator::Update` (`0x46d388`) skips advancing the clock (`globals[0, 5, 6]` and `world.ticks`).
   - Inside the loop (`0x46d625`–`0x46d653`), objects with scheduler bit 17 (`0x20000`, from `data[60] != 0`) are **not** skipped: when the countdown is positive, `0x46d63c` decrements it and skips `Simulate()` for that pass (`jmp 0x46d666`); once it reaches `0`, the next pass invokes `cXObject::Simulate()` (`0x46d653`). For counter `117` paused at tick `3189`, paused pass `16811` decrements `Idle(20000)` to `0`, paused pass `16812` wakes and enters `BHAV 4096` node 1 (`Idle(5)`), and paused pass `16817` re-enters `BHAV 4096` node 0 → `BHAV 8224 (Main)` → `BHAV 8195 (update just me)` and sets `data[0] = 1`.
   - Both `Counter - Kitchen - Moderate` (`114`–`117`, `BHAV 8223` node 7) and `NeighborhoodHelpSystem` (`id = 363`, `BHAV 4098` node 4) set `data[60] = 1`.
4. **Live capture timeline (`pid 14006`, Nov 20).**
   - The reference capture paused at `18:29:42` (`08:16:18`, tick `3189`, ~16,811 sleep ticks remaining on counters `114`–`117`, 1 tick behind on `NeighborhoodHelpSystem`).
   - `far-r0-cut-0816` (`18:31:28`), `far-r1-cut-0816` (`18:32:39`), `medium-r1-cut-0816` (`18:35:02`), and the pre-parking screenshot `pan1.png` (`18:41:04`, identical `(1760, -1968)` camera to `near-r3-down-0816`) were all shot before `Idle(20000)` expired: counter `117` is `DGRP #100` in all four, and our `near-r3-down-0816` (`paused_ticks = 0`) render matches `pan1.png` to 99.97% (`122` RGB565 differences across the `455,604` non-masked pixels after `near-r3-down-0816`'s `leave_out` rectangles; `5,121` across all `480,000` pixels including the pause banner, blinking pause button, and animated fountain), with `0` kitchen/counter/grass diffs.
   - Between `pan1.png` (`18:41:04`) and `near-r3-down-0816.png` (`18:41:40`, ~11.5–12 minutes after pausing, while only a mouse-park command ran), `Idle(20000)` expired while paused: counters `114`–`117` finished `Idle(5)` and ran `BHAV 8224` → `8195 (update just me)`, flipping counter `117` to `data[0] = 1` (`DGRP #101`) and refreshing their dirty rects (accounting for the 1-LSB 565 grass dither diffs below counters `114` and `117`). Seconds later, `NeighborhoodHelpSystem` also woke from `Idle(20000)` → `Idle(100)` and raised the `"Old Town Shuttle"` dialog (`pan2.png`), all while the clock remained frozen at `08:16:18` (`tick 3189`).
   - `near-r3-down-0816.png` (`18:41:40`) and `near-r0-up-0816.png` (`18:50:30`) captured this post-wakeup state.

## Decision

- **No production changes to `sims-vm`, `sims-render`, or `sims-game`.** `sims-vm` already implements both the load-time signature reset (`0x43a516` → `0x431fbe`) and paused partial scheduler passes (`0x46d553`, `crates/sims-vm/src/tick.rs`). Pausing `House05.iff` at tick `3189` in `sims-vm` and stepping partial passes reproduces the exact wakeup: `objects[&117].data[0]` flips from `0` to `1` at `paused_tick = 16817` with `world.ticks` and `(hour, minute, second)` unchanged, while straight counters `114`–`116` stay at `0`.
- **Add optional `paused_ticks` to the ratchet manifest (`crates/sims-game/tests/ratchet.rs` and `ratchet.txt`).**
  - `paused_ticks` is an optional `u32` (default `0`), valid only when `paused = yes`.
  - Each distinct `(tick, paused_ticks)` with `paused_ticks > 0` runs all its full and paused passes on a fresh isolated `Game` before any scene mutates camera or strip state, so neither paused-object wakeups nor earlier scene UI interactions can leak into later simulation passes (such as `night-far-r0-cut-2153` or another paused scene at a higher `paused_ticks`).
  - `near-r3-down-0816` and `near-r0-up-0816` set `paused_ticks = 16817`; `far-r0-cut-0816`, `far-r1-cut-0816`, and `medium-r1-cut-0816` stay at `paused_ticks = 0`.
- **Add a `sims-vm` regression test** (`a_paused_goth_house_wakes_counter_117_at_idle_20000` in `crates/sims-vm/tests/boot.rs`) pinning the exact `0x46d553` paused wakeup at `paused_tick = 16817`.

## Measured ratchet deltas (vs `0223004c`)

Only `near-r3-down-0816` and `near-r0-up-0816` change; all other 6 scenes keep identical frame SHA-256 hashes:

| Scene | `frame` | `surface` | `objects` | `people` | `panel` |
|---|---:|---:|---:|---:|---:|
| `near-r3-down-0816` | 98.40% → 99.77% (+1.37 pp) | 99.69% → 99.69% | 94.50% → 99.93% (+5.43 pp) | - | 99.88% → 99.88% |
| `near-r0-up-0816` | 99.94% → 99.98% (+0.04 pp) | 99.95% → 99.99% (+0.04 pp) | 99.93% → 100.00% (+0.07 pp) | - | 99.88% → 99.88% |
