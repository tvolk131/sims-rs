# `end` while a note plays fades out, keeps running, and runs on

*Decided 2026-11-21.*

Previously, when a HIT script reached `end` (or `return`) with an empty call
stack while its note was still playing, `sims-audio` ended the thread and let
the note play on at its last levels in a side table (`Player::ended`). That
diverged from the original's single-track-object model, required special-case
handling across placement, `Spl` lookup, muted replay, and the day/night loop,
and caused a replay of the track to start a second overlapping note.

## The original (Complete Collection 1.91)

- **Instruction fetch (`0x57e0e3`):** fetches the opcode byte at `[esi + 0x19c]`,
  advances `[esi + 0x19c]` by 1 (`next`), and jumps through the opcode table at
  `0x57f40f`. Both opcode `0x08` (`return`) and opcode `0x0c` (`end`) jump to
  `0x57e3ee`.
- **`end` / `return` (`0x57e3ee`):** checks the return stack `[esi + 0x1a8]`
  (`0x576899`). When non-empty, it pops the return address into `[esi + 0x19c]`
  and returns `al = 1` (`0x57f408`). When empty (`[ebp - 4] == 0`), it jumps to
  `0x57edc8`, which calls track vtable `+0x34` (`0x671a74 + 0x34 = 0x57dca3`)
  and returns `al = 1`.
- **Fade-out-and-kill (`0x57dca3`, vtable `+0x34`):**
  - Calls vtable `+0x70` (`0x57d567`, reading `[esi + 0x1c4]`, `is_running`).
    If false, returns `1` immediately (`0x57dcaf`) without touching any voice.
  - Checks `[esi + 0x1d0]` (the track's active voice) and calls its vtable
    `+0x70` (`is_playing`).
  - **Voice playing (`0x57dcc9`..`0x57dd0b`):** calls track vtable `+0x84`
    (`0x57de57`, which redirects through `target` at `[ecx + 0xc8]`) to write
    `FADE_DEST` (`0x28`) = `0`, `FADE_VAR` (`0x29`) = `EXTVOL` (`0x13`),
    `FADE_SPEED` (`0x2a`) = `100` (`0x64`), sets `byte [esi + 0x1cf] = 1`
    (`kill_after_fade`) directly on `esi`, and calls vtable `+0x84` to write
    `FADE_ON` (`0x2d`) = `1` (which also leaves positive comparison flags on
    the written track via `0x57de98`). Returns `1` without calling kill,
    without clearing `[esi + 0x1c4]` (`is_running`), and without moving the
    wait deadline `[esi + 0x198]`.
  - **No voice playing (`0x57dd0d`..`0x57dd11`):** calls vtable `+0x38`
    (`0x57dd3e`, kill), which clears `[esi + 0x1c4]`, cuts the voice
    (`0x57f625`), clears `[esi + 0xa8]` (`FADE_ON`) and `[esi + 0x1cf]`,
    writes `ISPLAYING` (`0x2e`) = `0` locally, clears `target`, resets PC to
    the entry point (`0x57dbe2`), zeroes `[esi + 0x198]` (wait deadline),
    removes the track from the running-track list (`0x57d050`), clears the call
    stack, and drops `main_duckpri` (`0x57d2cb`).
- **Track update loop (`0x57d7cd`):**
  - Steps any active fade (`0x57d7ff`..`0x57d86e`), then checks
    `[esi + 0x1cf] != 0 && [esi + 0xa8] == 0` (`0x57d86e`..`0x57d88d`) and
    calls vtable `+0x38` (`0x57dd3e`) to kill the track and return.
  - Runs the instruction loop (`0x57d8b3`..`0x57d927`) while
    `[esi + 0x198] != 0 && now >= [esi + 0x198]`. Because `0x57dca3`'s fade
    branch leaves `[esi + 0x198]` unchanged and `[esi + 0x19c]` already points
    at `next`, the loop immediately executes the bytes following `end` in the
    same update.
  - A track started again during that fade (`0x57d960`) still sees
    `is_running == 1`: `0x57d9b2`..`0x57d9eb` clears `kill_after_fade` and
    `FADE_ON`, resets `target`, and writes arguments; if paused/held,
    `0x57d9f1`..`0x57da12` resumes it; otherwise `0x57da26`..`0x57da34` checks
    `interrupt` (`0x37`) to decide whether to continue in place or restart.

## What changes here

- **`Player::step` (`Flow::End`):** when `thread.stack` is empty and any note
  of `thread.id` is still playing, writes `FADE_DEST = 0`, `FADE_VAR = EXTVOL`,
  `FADE_SPEED = 100` through `self.write` (`target`), sets
  `thread.kill_after_fade = true`, writes `FADE_ON = 1` through `self.write`,
  sets `thread.pc = next`, and continues the block's instruction loop. When no
  note of `thread.id` is playing, ends the thread at once.
- **Fault cleanup (`Player::fault`):** if execution faults (including running
  past the end of the program buffer after `end`, where the original would read
  past its heap buffer without bounds checks), stops the thread and cuts its
  notes so no orphaned note outlives its thread.
- **`Player::ended` removed:** because a script-started note can no longer
  outlive its running thread, `Player::ended` is dead and removed. Placement,
  `track_spl`, `camera_destination`, `apply_levels`, `day_night_playing`, and
  `fade_and_kill` now operate strictly on running threads (`self.threads`) and
  idle kind-53 voices (`self.idle`), matching the original's invariant.
- **`Player::fade_and_kill`:** no longer removes non-running notes (such as
  idle kind-53 voices), matching `0x57dca8`..`0x57dcaf` returning immediately
  when `is_running == 0`.

## Install check and remaining gaps

- A CFG scan of all 27 HIT files in the Complete Collection install (4,166
  `end` instructions, 1,305 `note_on` instructions) confirmed that zero
  routines in the install can reach `end` while a note is still playing. All 9
  install-backed `sims-audio` tests pass unchanged.
- **Unverified at run time:** running past `end` into trailing bytes is
  verified from the 1.91 disassembly (`0x57d8b3`, `0x57e3ee`, `0x57edc8`,
  `0x57dca3`), not captured in a live game run.
- **Still unported (unrelated to `end`):** `target` across a kill and fresh
  start (`0x57e023`), ducking across a restart, and the sound manager's 20 ms
  periodic update tick (`0x572338 → 0x572fba → 0x5edc57(0x14)` with the
  millisecond clock at `0x5fdb16`, calling `0x572fd6 → 0x57304c → 0x57cf37`
  every 20 ms rather than 1 ms; currently `sims-audio` steps once per 64-frame
  block ≈ 2.9 ms).
