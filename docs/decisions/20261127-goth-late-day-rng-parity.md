# The late Goth day's lot stream matches under the frames the original drew

*Decided 2026-11-27.*

- **Status:** Accepted
- **Scope:** `crates/sims-vm/tests/integration/boot.rs` (tests only; no simulation code changes)
- **Supersedes in part:** the "lot stream through 36,717" status of `20261120-a-step-runs-both-ways-while-a-refresh-is-pending.md`, and the frame fit of `20261118-goth-day-frame-schedule.md` from 28,315 on

## Context

The roadmap said that on the 43,200-update Goth day the C runtime stream matched the original at every update, but the lot stream only through update 36,717. The earlier full-day audit listed the lot stream's first moves after that: one update early at 36,718, 36,929, 37,058, 37,126 and 37,215. Comparing the two recordings of the day adds 41,445 and 41,893, where the raccoon draws in one and not the other.

Those runs used the frame schedule fitted to `goth-day-20261115`, which watched no frames:
- after updates 0, 1, 2 and 3, and every 30th from 33 through 4,143;
- then every 30th from 4,154, which is phase 14.

Track 2's `goth-day-frame-20261121` is a second recording of the same day. It watches the draw guard (`frame=0x6f0f48:1`, set at 0x4d1b60 and cleared at 0x4d1be7), so it records the frames the original actually drew: 1,512 of them, drawn between updates.

## Findings

**1. Under the frames the original drew, the port matches the whole recorded day.**
- The replay draws a frame after update 0 and after each update the recording drew one after.
- It uses `--answers 3,3` and `--view-centre 32,32`.
- On `af9733f8d`, the lot stream and the C runtime stream equal the recording's after every one of its 44,200 updates.
- The full state was compared at all 938 updates the recording saved. The comparison covers each object's data, temps, attributes, stack frames and person data, the globals, the scheduler cursor and the three random streams.
  - It follows the day audit's masking: person words the port lists as unknown in routing (`unknown_words`) and frame locals it marks unknown are skipped. Unmasked, the extra differences are all person word 37, which the port marks unknown (for example object 12 at 34,000: 3,096 in the original, 0 here).
  - From 28,400 to 44,200 the only differences are the global stream and update 40,495.
  - 40,495 is a dialog save. The recording took it inside the update, while the dialog was up, and drew in-update frames there. Global 18 is still 760 at that moment and -1 by 40,496; the port, saved after the whole update, has -1. At 40,496 the two agree.
  - The global stream differs from 4,155, which is taken to be the censor mosaic (`20261118-goth-day-global-draws-are-censor-mosaic.md`).
  - Earlier checkpoints still differ; see Left open.

**2. The five early draws come from the fitted schedule.**
- From 28,315 to 39,595 the original drew its frames at phase 25, not 14.
- Under the fitted phase-14 schedule, the lot stream moves at exactly 36,718, 36,929, 37,058, 37,126 and 37,215, each exactly one update early: the value there is the original's at the next update. Nowhere else changes, from 1 to 44,200.
- A frame counts the rooms and lists the selected Sim's queue, which clears flag 0x80 (`20261118-goth-day-frame-schedule.md`). Which update a frame follows is therefore an input of the run.
- There is no simulation rule to port.

**3. At the raccoon's 41,445 and 41,893 the two runs of the original disagree.**
- `goth-day-20261115` and `goth-day-frame-20261121` are two runs of the original. Their lot streams differ at exactly these two updates.
- The port matches the frame-watched recording at both, under either schedule.
- The frame-watched recording shows an out-of-update animator refresh nearby:
  - After update 40,330, both Mortimer's and the raccoon's animator clocks (person +0xadc, +0xb4) are written at 0x58da4a, inside Refresh(0) at 0x58da12.
  - The call stack is 0x4b348c ← 0x5255c1 ← 0x5250e1 ← 0x5dd0b5 ← 0x5dd0b5 ← 0x5d7dc7 ← 0x5ef8b8 ← 0x5d6018 ← 0x638444. That is a paint from the message loop, between updates.
  - The refresh moves each clock one update's step early (0x623efa → 0x623f2c).
  - Update 40,331's own clock write (0x58dcc3) then stores the same value. The watch shows only writes, so this establishes that the values agree from that write on. A read of the clock between the refresh and that write would see the advanced value; the watch does not exclude one. Since the port matches the streams at every update here, no such read moves them.
- **Hypothesis, not shown:** in `goth-day-20261115` the paint fell at another moment of wall-clock time, and the raccoon drew one update early. That recording watched neither frames nor animator clocks, so the cause of its early draws is not established. What the recordings do establish is that the original's two runs differ in the lot stream at exactly these two updates.
- A deterministic port can follow one run of the original, not both. It follows the recording that watched its frames, and the port matches it at every update.

## Decision

- No simulation code changes. The Goth `TICK_43200` and every other strict pin are unchanged, since the default-frames run does not move.
- New test `the_goth_day_under_the_frames_the_original_drew_keeps_its_streams`:
  - replays the Goth day under `GOTH_DAY_FRAMES`, the recording's 1,512 frames plus the one before update 1, stored as 60 runs of every-30th updates;
  - checks the lot and C runtime streams at 24 named updates, including the five above, 40,330/40,331 and 41,445/41,446/41,893;
  - checks the SHA-256 of both streams after every one of the 44,200 updates against the recording's (`GOTH_DAY_STREAMS`).
  - Moving the phase-25 stretch to phase 14 fails it at 36,718.
- The existing fitted-schedule test is kept. Its comments now give the causes above instead of the unmodelled paint the earlier note inferred.

## Range

| | lot stream | C runtime stream |
| --- | --- | --- |
| before (roadmap, fitted frames) | 1 to 36,717 | 1 to 43,200 |
| now, frames the original drew | 1 to 44,200 (every update) | 1 to 44,200 |

## Left open

These are state differences the random streams do not show. All but the last are at saved updates before 28,400, with the masking above; the last is what the masking hides:
- object 21's `data[50]` is 0 in the original and 1 here at 173 saved updates from 5,268 to 18,200, though not at all of them (both are 1 at 18,007, for example);
- global 11 is 1 in the original and 0 here at 12,986, 18,007 and 19,194;
- uninitialised locals: object 18's frame 9 at 368 and 400, and object 21's frame 7 at 19,400 to 19,436;
- the speed globals 16 and 18: the port drops to idle speed, (1,000, 760), at 28,195. The original is still at (760, -1) at 28,200 and at (1,000, 760) by 28,317;
- person word 37, which the port marks unknown, differs at many saved updates (object 12 from 34,000, object 10 from 34,382, object 21 at 41,711 to 41,875).

None of these moves a stream. They are next for the early-day audit.
