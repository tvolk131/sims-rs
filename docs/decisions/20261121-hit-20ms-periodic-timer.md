# HIT 20 ms (50 Hz) Periodic Sound Timer

## Context

Previously, `sims-audio` stepped HIT track scripts and fades once every `BLOCK` (64 frames at 22,050 Hz, ~2.90 ms) while stepping the kind-25 volume-nudge countdown (`0x573019`) on the 20 ms (`TIMER_FRAMES = 441` frames, 50 Hz) timer. That left all HIT fades running ~6.9× faster than Complete Collection 1.91 (for example, `FadeAndKillTrackSlow`'s `fade_speed = 3` finished in ~0.99 s instead of ~6.84 s), treated `wait` operands as milliseconds instead of 20 ms HIT ticks, polled `wait_samp`/`waiteq..waitle`/`seqgroup_kill` at 64-frame boundaries instead of 2 HIT ticks (40 ms), and yielded a full block on `loop`.

## Disassembly Findings (Complete Collection 1.91)

- **Periodic timer (`0x572fc3` → `0x572fd6`):**
  The sound manager installs a 20 ms (`push 0x14`, exactly `TIMER_FRAMES = 441` frames at 22,050 Hz) periodic timer. Callback `0x572fd6` checks the initialized flag `[0x6f640c]`, runs the kind-25 volume-nudge countdown (`0x573019..0x573038`), increments `[manager + 0x14]` (`0x57303a..0x573043`), and calls `0x57cf37` on the HIT engine (`[0x6f6ac8]`).
- **HIT engine tick (`0x57cf37..0x57d016`) and clock (`[hit + 0x58]`):**
  The HIT constructor (`0x57c9b8..0x57c9c5`) initializes `[hit + 0x58] = 1`, `[hit + 0x54] = 1`, and `[hit + 0x60] = 1` (so a wake tick of `0` unambiguously means stopped/inactive). On each 20 ms tick, `0x57cf37` increments `[hit + 0x58]` by `1` (`0x57cf74`), snapshots the running track list (`[hit + 0x2c]`), and calls `vtable[+0x1c]` (`0x57d7cd`) on every running, unheld track.
- **Unpause clock increment (`0x57cd2c`):**
  Before walking the registry to call `let_go` (`vtable[+0x5c]`, `0x57dc63`, setting each track's wake tick `[esi + 0x198] = [hit + 0x58]`), `0x57cd3e..0x57cd46` increments `[hit + 0x58]` by `1`.
- **Track tick (`0x57d7cd..0x57d95f`):**
  1. Returns immediately if `!running` (`[esi + 0x1c4] == 0`) or `held` (`[esi + 0x1c8] != 0`).
  2. Steps any active fade once per 20 ms tick (`0x57d7ff..0x57d868`), and kills the track (`0x57dd3e`) if `kill_after_fade` (`[esi + 0x1cf]`) is set and `fade_on == 0` (`0x57d86e..0x57d88d`).
  3. Runs instructions while `wake != 0 && hit_clock >= wake` using unsigned comparison (`jb` at `0x57d8c0`).
  4. Checks timed `note` auto-stop (`0x57d929..0x57d951`): if `[esi + 0x194] != 0 && hit_clock >= [esi + 0x194]`, stops the active voice (`vtable[+0x98]`) and clears `[esi + 0x194] = 0`.
- **Opcode timing:**
  - `note` (op 1, `0x57e145..0x57e18d`): reads key and duration registers (`0x57f561` clamps `> 127` to `1`), arms `[esi + 0x194] = [hit + 0x58] + duration`, and calls `note_on` (`0x57f595`).
  - `wait` (op 9, `0x57e42d..0x57e459`): validates register (`0x57f561` clamps `> 127` to `1`; register `0` logs a fault via `0x57f271` without updating `[esi + 0x198]`), reads the register in **20 ms HIT ticks** (not milliseconds; verified against `SimsSound.hit` `MainMusicLoop` `loadl r5, 250; wait [5]` = 5.0 s and `loadl r5, 50; wait [5]` = 1.0 s), and writes `[esi + 0x198] = [hit + 0x58] + ticks`. A `0`-tick wait leaves `wake == hit_clock` and continues executing in the same tick.
  - `wait_samp` (op 11, `0x57e468..0x57e485`), `waiteq..waitle` (ops 77..82, `0x57e48a..0x57e6cd`), and `seqgroup_kill` (op 69, `0x57eef3..0x57efe6`): when blocked, keep `pc` on the instruction and set `[esi + 0x198] = [hit + 0x58] + 2` (`inc eax; inc eax` = **2 ticks = 40 ms = 882 frames**).
  - `loop` (op 32, `0x57ee33..0x57ee3b`): jumps to `loop_point` in the same tick without touching `[esi + 0x198]` (all 142 `loop` instructions in the Complete Collection install are guarded by `wait`/`wait_samp`/`wait*`).
- **Ambience cadence (`cFreshPlayer::Update`, `0x577ab9`):**
  Ambience updates run per frame in 1.91 (not on the 20 ms HIT timer) and compare `now - due > 7` ms (`ambience.rs:608`), so keeping `step_ambience` on `BLOCK` (64 frames, ~2.9 ms) preserves exact ambience beat scheduling.

## Decisions

1. **Drive HIT track/fade execution from the 20 ms (`TIMER_FRAMES = 441`) timer in `crates/sims-audio/src/player.rs`:**
   - Maintain `Player::hit_clock: u32` initialized to `1`, incremented once per `TIMER_FRAMES` tick in `step_timer` (right after `0x573019`'s nudge countdown) and once in `unpause_tracks` (`0x57cd46`).
   - Switch `Thread::wake` from frame units (`u64`) to HIT clock ticks (`u32`), add `Thread::note_off: u32` (`[esi + 0x194]`), and check `note_off` auto-stop at the end of `step` and `out_of_budget`.
   - Port opcode 1 (`note`), switch `wait` to 20 ms HIT ticks with `0x57e434` register validation, set `Flow::Block` (`wait_samp`, `waiteq..waitle`, `seqgroup_kill`) to sleep `POLL_TICKS = 2` ticks (40 ms), and remove the artificial yield from `loop` (retaining `Flow::JumpYield` only for host safety on self-restarting `play_trk`).
   - Advance `render_stereo` and `render_until_idle` to `until_timer.min(until_block).min(remaining)` so HIT ticks stay on exact 441-frame boundaries while ambience updates remain on `BLOCK = 64` boundaries.
2. **Update tests and pinned install digests:**
   - Update `crates/sims-audio/tests/player.rs` from `BLOCK` (64-frame) tick assumptions and bare `[SET_LOOP, LOOP]` idioms to `TIMER_FRAMES` (441-frame) ticks, 20 ms `wait` units, and `POLL_TICKS = 2` boundaries, and add dedicated regression tests for 20 ms fade stepping, `wait 0` / bad register, timed `note`, zero-gap `loop`, and `unpause` clock increment.
   - Update `crates/sims-audio/tests/install_player.rs` (`the_phone_rings` 58,212 frames = `66 * 882` at the 40 ms `wait_samp` poll; `a_sim_takes_a_soft_footstep` same chosen patches with 40 ms tail; `the_backyard_ambience_plays` same ambience cursor `(13, 6)` and active count `3` with 6.84 s neighbourhood music fade-out; `every_event_definition_runs_without_a_bad_instruction` 183 still-running events after 20 HIT ticks = 400 ms with 0 unported instructions and one new step-limit fault on `playtag_laugh_vox` track 3928, whose child-only `call 0x103; loop` genuinely spins without waiting under `NoSources` gender `0`) and `crates/sims-game/tests/sound.rs` (`pausing_the_lot_pauses_live_ambience_and_night_loop` draining `bkground_live`'s 6.84 s slow fade of `PAUSE_SPARES` `MainMusicLoop` while paused).
