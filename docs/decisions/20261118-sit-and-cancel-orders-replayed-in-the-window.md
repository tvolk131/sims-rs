# The Sit and cancel orders replay in the window; what the pointer finds still differs

*Decided 2026-11-18.*

Two more of track 2's player-order recordings of the Goth house are now
replayed through the window's input layer, like the snack order
(`20261115-replay-draws-recording-frames-queue-listing.md`).

- `goth-sit-read-order-20261115`: Mortimer is ordered onto the porch
  bench (Sit). The second order, Read A Book on the bookcase, never
  opened in the original. One possible cause is that its press fell
  inside one of the harness's stops (unverified).
- `goth-cancel-order-20261116`: two snacks are ordered from the fridge.
  A press on the queued snack's icon takes it off, then a press on the
  running snack's icon cancels it.

The tests are `the_recorded_sit_order_replayed_in_the_window_matches_the_recording`
and `the_recorded_cancel_order_replayed_in_the_window_matches_the_recording`.

## How the input is given

- **Where.** At the recording's points, moved 97 pixels right and 46
  down. That is the snack order's camera offset. The Sit recording's
  screenshot at updates 234–256 puts Bella at the same offset, and the
  panel does not move. Pie entries are found by label: the original's
  pie for the fridge is moved onto the screen, ours is not
  (`a_menu_near_the_edge_moves_onto_the_screen_as_the_original_moved_it`).
- **When.** Each harness command runs over many updates, because the
  game stops at every update while the state is saved. Mostly the
  input's effect first shows in the state of the update its command
  ended, so the replay gives it once the lot has run the update before
  that. The recording's state shows when:
  - Mortimer's last result shows when a hover or press listed a pie.
  - The queue shows when an order or a cancel took effect.

  The one exception is the second cancel. Its button-down command
  ended at 593 and its release command ran from 595 to 625, yet update
  595's state, saved before the release was sent, already shows the
  cancel. Which of the two button events acted is not known; the replay
  presses before 595, where the effect shows. The order recorded again
  with the pick watched does not repeat this: both its cancels act at
  their release's end
  (`20261120-cancel-order-replayed-with-the-pick-watched.md`).
- **A press acted on its release where the timing shows it.** The
  cancel recording sends the button down and up as separate commands:
  - The choice of Have A Snack took effect at the release's end (257
    and 445), as did the first cancel (505). The second cancel does not
    fit this (above).
  - The pie opened with the press given at the release (183). A press
    at the button-down was not tried.
- **Frames.** The frames the original drew are an input of each
  recording, settled from when the Room motives took a room's new
  score:
  - Sit/Read has a frame after every 30th update from 170 (after 470
    and 620).
  - Cancel has frames after 316, 474, 648 and 1124. The first two are
    where a pointer move reached the game.

  Frames that change nothing in the state are not known. The tests keep
  the default's frames before update 171.

## Verified

- **Cancel.** With the animator reset that removes a playing clip
  (#263), the lot equals the recording's after every update from 1 to
  1300, except for two differences taken for listings the original's
  pointer made and ours did not (below). Since then, the one at 90 is
  settled by drawing on the recording's frames, and the one at 257–259
  by the second check of a chosen entry
  (`20261120-choosing-a-pie-entry-checks-it-again.md`): the cancel
  replay now equals the recording at every update to 1300. That covers:
  - the second order queued behind the first;
  - the press on its icon taking it off;
  - the press on the running snack's icon setting its flag 0x100,
    queueing his own action 8318 (table index 33) and clearing person
    word 33;
  - the salesman leaving at 597.
- **Sit.** The lot equals the recording's after every update to 260,
  except for Bella's local from 203 to 227, as in the snack order. The
  Sit order is pushed before update 201 and is running, with an empty
  queue, as recorded.

Neither comparison checks two of the salesman's words that our state
marks as unknown, both from update 14: his person word 37 (3291
recorded, 0 ours) and local 7 of his third frame, tree 0x1003 (2609
recorded, 0 ours).

## Differences: what the pointer finds

These are all **unverified** and not ported. The lot's state shows only
that the original listed a pie where ours did not, or listed another
one.

- **Through a wall.** At update 90 the pointer at the recording's
  (176, 222) makes the original list a pie: Mortimer's last result
  changes at 91. The same hover in the write-watch recording
  (`goth-snack-d50-watch-20261115`) lists the counter, object 119. The
  screenshot before the order shows the kitchen's front wall standing
  there, with Mortimer outside. Ours finds nothing there: our frame's
  wall pixels name nobody, so they hide the counter
  (`crate::pick`'s module doc). Later in the same run, with Mortimer in
  the kitchen, the same point finds 119. The snack order differs the
  same way at 88, but its test gives its hovers by object, not by
  point.
- **Under the still pointer.** After a choice the pointer stays on the
  pie entry, and these differences were taken for pies the original
  listed there while the lot ran:
  - in cancel, after 256: for 257–259 the original's word 50 is 0
    where ours was 1 before the check below was ported, and its last
    result 1 where ours was 0. Ours now matches. The release's second
    check of the entry accounts for these
    (`20261120-choosing-a-pie-entry-checks-it-again.md`, from the
    snack-bob recording's watch). A listing at the same updates is not
    ruled out for this run; in the order recorded again with the
    listed object watched, the original listed nothing under the still
    pointer after either choice
    (`20261120-cancel-order-replayed-with-the-pick-watched.md`);
  - in Sit/Read, at 261 (a failed check) and at 291 (word 50 goes to 1
    until 560);
  - in the snack order, at 622, which the write watch shows was the
    plumb bob, object 383 (`20261118-plumb-bob-picks-as-itself.md`).

  Ours lists nothing. Our bob is drawn 10 to 30 pixels below and right
  of the pointer in the snack order and in Sit/Read. In cancel at 257
  it is about 190 pixels away, so the bob's geometry could not explain
  that difference, which the release's check accounts for. Track 2's queued snack recording (screenshots during
  the wait) will also keep the word-50 write watch. The watch's saved
  arguments name the object listed, and the screenshot shows where it
  is drawn.
- **The salesman or the bookcase.** At 590 the pointer at the
  recording's (384, 250) found the bookcase (132) in the original,
  according to the harness's hover log. Ours finds the salesman (259):
  he is drawn at (481..490, 273..311) over the point, and our
  bookcase's pixels end 5 rows above it.
- **What follows.** The Sit run's other differences (word 50 at 591–620
  and 651–747, Mortimer's last result at 651–698, a local of his tree
  8301 at 339–356) follow from these listings.

The replayer and comparer used to find these are scratch tools, not in
the repository.
