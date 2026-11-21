# The cancel order recorded again with the pick watched replays in the window

*Decided 2026-11-20.*

Track 2 recorded the two-snack cancel order of `goth-cancel-order-20261116`
again (`goth-cancel-pick-20261120`), with two write watches:

- the pointer pick's word at 0x6f6bc4;
- the listed object, the two bytes at +0x2e of 0x6e5528.

It has extra screenshots after the first choice and around the second. The
replay is
`the_recorded_cancel_order_with_the_pick_watched_replayed_in_the_window_matches_the_recording`.

The recording ends at update 845, before its last step: the scenario's
`wait 200` from 656 and its final screenshot did not finish. Everything
the replay checks lies before that.

## What the watches show

- **Frames.** The pick word is written once per drawn frame (0 each time,
  from 0x58463e, called from 0x4a8deb). Its updates give the frames:
  `0,1,2,3,33,63,93,123,153,166,188,210,240,270,300,330,360,390,411,431,446,476+30`.
- **The original picks only at frames.** Every write of the listed object
  (from 0x48c8dd) falls on a frame's update:
  - 123: the fridge (118), after the move onto it;
  - 153: the fridge again, written twice by the press;
  - 240: nothing;
  - 300: the kitchen counter (119), after the move to the recording's
    (176, 222);
  - 330: nothing;
  - 360: the fridge;
  - 390: the fridge again, written twice by the press;
  - 476: nothing.

  The writes at 240 and 476, under the still pointer after each choice,
  come from the same path as the moves' writes (0x48ca31 from 0x476bf9).
- **Nothing under the still pointer.** After each choice the original
  listed no object. The old cancel note did not rule out a listing at
  257–259 (`20261118-sit-and-cancel-orders-replayed-in-the-window.md`).
  That listing would have been under the still pointer after the first
  choice. In this run, under the same pointer and pie, the original listed
  nothing after either choice. That fits the release's second check
  accounting for those updates by itself
  (`20261120-choosing-a-pie-entry-checks-it-again.md`). It is not a watch
  of the old run.
- **The pick word.** Track 2 watched 0x6f6bc4 for the pointer pick. It
  never changes here, so it does not say which object a pick tested; the
  listed object does.

## How the input is given

Window points are the recording's moved 97 right and 46 down; the panel's
icons are at the recording's own points.

- **Pointer moves.** Each move is given before the frame that listed what
  it found:
  - 84;
  - 91: within the command's 88–94, before the frame after 93, which
    listed nothing;
  - 123, 300, 330 and 360.
- **Presses on the fridge.** At 153 and 390, the frames whose listing the
  press wrote.
- **Pie entries.** Have A Snack is reached at 166 and 411: Mortimer's last
  result goes to 1 at 167 and 412.
- **Choices.** At 210 and 446. The order shows at 211 and 447, the
  updates each release's command ended in.
- **Icon presses.** On the queued snack's icon at 536 and on the running
  snack's icon at 626. The queue empties at 537, and the cancel queues his
  8318 at 627: both at their release's end.

  The old run's second cancel showed before its release was sent
  (`20261118-sit-and-cancel-orders-replayed-in-the-window.md`). This run
  does not repeat that.

## Verified

The lot equals the recording's after every update from 1 to 845 (scratch
tool) except for Bella's local from 203 to 227, as in the other replays.

Our pointer lists what the original listed at every move. With the moves at
the recording's points, the counter shows at (176, 222) once a frame has
cut the kitchen wall away, as in the old cancel replay.

The test checks:

- the listings;
- Mortimer's last result around each listing and entry;
- the two snacks (uids 1 and 5);
- the queued one taken off;
- the running one's flag 0x100;
- his 8318 queued and his person word 33 cleared;
- Bella's Room motive at 477 and Mortimer's at 777;
- that nothing is caught to 845.

## Open

The Sit recording with screenshots (`goth-sit-shots-20261120`) stopped at
update 125, before its first Sit screenshot. It does not cover 261, 291 or
590, and needs recording again.
