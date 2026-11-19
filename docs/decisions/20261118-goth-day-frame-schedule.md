# The Goth day recording's frames drift against its updates

*Decided 2026-11-18.*

`Frames::default` (a frame after updates 0, 1, 2 and 3, after 33 and
every 30th from there) is the schedule three captures of the Goth house
showed. The long recording `goth-day-20261115` was made without a watch
on the draw guard, so its frames are not recorded. They show only where
a frame changes the state: a frame counts the rooms that wait to be
counted (0x4a8895), which each person's Room motive (motive 13) reads,
and it lists the selected Sim's queue, which clears an action's flag
0x80 (see `20261115-replay-draws-recording-frames-queue-listing.md`).

**The fit.** For each window where the port's motives or flags first
left the recording, the port was run with a frame every 30 updates at
each phase (the update number modulo 30) from the window on, and the
phases that matched the recording through the window were kept
(the last two rows are from the random streams, not motives):

| Window (update) | Phases that match |
|-----------------|-------------------|
| to 600 | 3 (the default) |
| 902 | 1 |
| 1060 | 20 to 29 |
| 2110 | 20 to 29 |
| 2920 | 11 to 18 |
| 3370 | 11 to 20 |
| 4155 | 14 if the global draws are the censor mosaic's, drawn in frames (unverified; 15 passes the stream checkpoints too; a row is saved before the frame after its update, so the draws in row 4155 come from the frame after 4154) |
| 5490 | 14 and 15 match, 3 (the default) does not (the lot stream; other phases not tried) |

So the original's frames are not every 30 updates: the phase moves
backwards and forwards over the day, as the main loop's timing allows.
The frame schedule is an input of a recording, like the pointer.

**What the port does.** Nothing changes in the strict boot, which keeps
the default schedule. The new test
`the_goth_day_under_the_recordings_frames_keeps_its_streams` uses phase 3
through 4143 and phase 14 from 4154, the two stretches the streams
constrain,
and finds the lot and C runtime streams the recording's at 4154, 5490,
6756, 10358, 13960 and 16111. The windows between (902 to 3370) change
motives and flags but no stream, so the test does not need their phases.

**Recommendation.** A capture meant for comparison should watch the
draw guard (`--watch frame=0x6f0f48:1`: set at 0x4d1b59, cleared at
0x4d1be7 in `DDDSimsView::Draw`, 0x4d1b37), so that its frames can be
replayed exactly.

**Unverified.** The phases are fitted, not seen. A phase range means
any of those phases matches in that window. The fit runs the port, whose
`Lot::step` gives the state after the frame that follows the update, so
its phases are in the port's terms (`Frames::after`).
