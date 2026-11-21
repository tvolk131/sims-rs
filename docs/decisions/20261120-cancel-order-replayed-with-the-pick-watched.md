# The cancel order recorded again with the pick watched replays in the window

*Decided 2026-11-20.*

Track 2 recorded the two-snack cancel order of `goth-cancel-order-20261116`
again (`goth-cancel-pick-20261120`), with two write watches:

- `pick`, the word at 0x6f6bc4;
- `tool`, two bytes the capture armed as `0x6e48bc@0@0x18@0@0x18+0x2e:2`
  (resolved to 0x0a420cee). They are taken here for the object the
  pointer listed: their values are 118 (the fridge), 119 (the kitchen
  counter) and 0, at the updates our pointer lists the same objects.

It has extra screenshots after the first choice and around the second. The
replay is
`the_recorded_cancel_order_with_the_pick_watched_replayed_in_the_window_matches_the_recording`.

The recording ends at update 845, before its last step: the scenario's
`wait 200` from 656 and its final screenshot did not finish. Everything
the replay checks lies before that.

## What the watches show

- **Frames.** Outside the updates, 0x58463e (called from 0x4a8deb)
  writes 0 to the pick word once per drawn frame; four writes from other
  instructions come at update 0. Those writes give the frames, through
  the last at 836:
  `0,1,2,3,33,63,93,123,153,166,188,210,240,270,300,330,360,390,411,431,446,476+30`.
- **The listed object.** Every write to it (from 0x48c8dd) shares its
  update with a frame:
  - 123: the fridge (118), after the move onto it;
  - 153: the fridge again: the press clears it, then writes 118;
  - 240: nothing;
  - 300: the kitchen counter (119), after the move to the recording's
    (176, 222);
  - 330: nothing;
  - 360: the fridge;
  - 390: the fridge again, cleared and written by the press;
  - 476: nothing.

  The writes at 240 and 476, under the still pointer after each choice,
  come from the same path as the moves' writes (0x48ca31 from 0x476bf9).
  The watch shows writes, not picks. It does not show that every pick
  happens while a frame is drawn, or whether a pick ran at 93, where
  nothing was written.
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
  is only ever 0 here, so it does not say which object a pick tested.

## How the input is given

Window points are the recording's moved 97 right and 46 down; the panel's
icons are at the recording's own points.

- **Pointer moves.** The first is given at 84, where its command ended.
  The second is given at 91, within its command's 88–94 and before the
  frame after 93: our frame there shows the counter at the first move's
  point, and the original wrote no listing at 93. Where the original's
  pointer was at that frame is not known. Each later move is given at the update of the listing
  it made: 123, 300, 330 and 360. The window draws that update's frame
  first, then takes the move. The move onto the fridge ended at 122; his
  last result goes to 0 at 124.
- **Presses on the fridge.** At 153 and 390, the updates of the listing
  each press wrote.
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

In every word the scratch comparison checks, the lot equals the
recording's after every update from 1 to 845, except for Bella's local
from 203 to 227, as in the other replays. The comparison skips words our
state marks as unknown.

Our pointer lists what the original listed at every move. With the moves at
the recording's points, the counter shows at (176, 222) once a frame has
cut the kitchen wall away, as in the old cancel replay.

The test checks:

- the listings;
- Mortimer's last result around the first order's listings and both
  entries;
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
