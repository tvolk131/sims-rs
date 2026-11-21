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
  `0x57edc8`, which calls track vtable `+0x34` (slot `0x671aa8` in vtable
  `0x671a74`, containing `0x57dca3`) and returns `al = 1`.
- **Fade-out-and-kill (`0x57dca3`, vtable `+0x34`):**
  - Calls vtable `+0x70` (`0x57d567`, reading `[esi + 0x1c4]`, `is_running`).
    If false, returns `1` immediately (`0x57dcaf`) without touching any voice.
  - Checks `[esi + 0x1d0]` (the track's single active voice) and calls its
    vtable `+0x70` (`is_playing`).
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
    writes `ISPLAYING` (`0x2e`) = `0` locally, clears `target`
    (`[esi + 0xc8] = 0`), resets PC to the entry point (`0x57dbe2`), zeroes
    `[esi + 0x198]` (wait deadline), removes the track from the running-track
    list (`0x57d050`), clears the call stack, and drops `main_duckpri`
    (`0x57d2cb`).
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
- **Related single-track-object behaviors verified in 1.91:**
  - **`target` (`0x57e018`..`0x57e036`, `0x57de57`, `0x57e059`..`0x57e064`):**
    stores the registry object pointer from `0x573312` regardless of whether
    the destination track is running. When the destination has stopped, writes
    still route there rather than falling back to the caller, and writing
    `FADE_ON` (`0x2d`) clears `[dest + 0xa8]` back to `0` because
    `is_running` is false.
  - **Single active voice (`0x57f595` → `0x58015a`..`0x58018d`) and `note_off`
    (`0x57e192`..`0x57e1a9` → `0x5801a0`):** each track owns one active voice;
    `note_on` restarts it and `note_off` stops it unconditionally (ignoring
    the raw operand byte).
  - **Pause (`0x574869`..`0x5748c9`, `0x57dc3d`):** snapshots only the
    running-track list (`[0x6f6ac8 + 0x2c]`), and hold checks `is_running`
    before pausing the voice, so idle kind-53 voices are not silenced.
  - **Refused `getsrcdatafield` (`0x57f33a` → `0x57f3f9` → `0x57f408`):** logs
    the error and returns to the instruction loop with the active voice intact.
  - **`play_trk` `SOURCE` write (`0x57f261`):** writes `SOURCE` (`0x2f`) to the
    started track through vtable `+0x84` (following its `target`, which matters
    on a muted start that leaves `target` unchanged).

## What changes here

- **`Player::step` (`Flow::End`):** when `thread.stack` is empty and any note
  of `thread.id` is still playing, writes `FADE_DEST = 0`, `FADE_VAR = EXTVOL`,
  `FADE_SPEED = 100` through `self.write` (`target`), sets
  `thread.kill_after_fade = true`, writes `FADE_ON = 1` through `self.write`,
  sets `thread.pc = next`, and continues the block's instruction loop. When no
  note of `thread.id` is playing, ends the thread at once.
- **Fatal vs. non-fatal faults (`Player::fault` vs. `Player::report_fault`):**
  fatal execution faults (including running past the end of the program buffer
  after `end`, where the original would read past its heap buffer) stop the
  thread and cut its active note; refused `getsrcdatafield` queries call
  `report_fault` so the active note stays intact.
- **`Player::ended` removed:** because a script-started note can no longer
  outlive its running thread, `Player::ended` is removed. Placement,
  `track_spl`, `apply_levels`, `day_night_playing`, `fade_and_kill`, and
  `pause` operate strictly on running threads (`self.threads`) and idle
  kind-53 voices (`self.idle`).
- **Track-object `target` (`Target::Track` / `Destination`):** `target` now
  stores the registered track ID (`0x57e023`), routing reads, writes, fades,
  placements, and `Spl` lookups through the destination track even when it is
  stopped (forcing `FADE_ON = 0` on stopped destinations at `0x57e059`) and
  reaching it again if it restarts.
- **Single voice per thread and `note_off`:** `note_on` replaces any prior
  note of the same thread (`0x58015a`) and `note_off` stops that voice
  unconditionally (`0x5801a0`).
- **Pause and `fade_and_kill` on idle kind-53 voices:** neither holds nor
  stops non-running idle kind-53 voices (`0x574869`, `0x57dc3d`, `0x57dca8`).
- **`play_trk` `SOURCE` write:** routes through `write_at` (`vtable +0x84`,
  `0x57f261`) so a muted start preserves the destination's `target`.

## Install check and remaining gaps

- A CFG scan of all 27 HIT files in the Complete Collection install (4,166
  `end` instructions, 1,305 `note_on` instructions) confirmed that zero
  routines in the install can reach `end` while a note is still playing. All 9
  install-backed `sims-audio` tests pass unchanged.
- **Unverified at run time:** running past `end` into trailing bytes is
  verified from the 1.91 disassembly (`0x57d8b3`, `0x57e3ee`, `0x57edc8`,
  `0x57dca3`), not captured in a live game run.
- **Update tick period (affects `end` fade duration):** the original's sound
  manager arms a 20 ms periodic timer (`0x572fc3 → 0x5edc57 → 0x6073f8`,
  using the millisecond scheduler clock at `0x5fdb16`), firing `0x5edd29 →
  0x572f22 / 0x572ea9 → 0x572fd6 → 0x57304c → 0x57cf37` at 50 Hz. HIT's
  deadline clock at manager `+0x58` counts enabled ticks at `0x57cf74`.
  Because `sims-audio` still steps once per 64-frame block (≈ 2.9 ms), an
  11-step speed-100 fade from full volume takes ≈ 32 ms here vs. ≈ 220 ms in
  1.91. Changing the VM step rate is left for a dedicated PR.
- **Other inherited differences left out of scope:** ducking across a restart,
  and same-update timing when `play_trk` restarts its own track with
  `interrupt` set (`0x57f267` returns directly to `0x57d8b3`, whereas here the
  restarted routine runs from the top on the next block).
