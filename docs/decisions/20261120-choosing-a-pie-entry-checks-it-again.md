# Choosing a pie entry checks it again; the snack order with plumb-bob shots replays in the window

*Decided 2026-11-20.*

## The second check

Choosing a pie entry now checks its action again before the order is
made. Until now the entry was checked only when the pointer reached it
(`20261115-press-lists-pie-menu-again-reaching-entry-checks.md`). The
Read note left this open
(`20261120-read-order-replayed-in-the-window.md`).

- **Seen in the original.** Track 2's snack recording with plumb-bob
  screenshots (`goth-snack-bob-20261120`) keeps the write watch on
  Mortimer's word 50, with return addresses. The house view's entry
  check (0x48df28) ran from its call at 0x51bb9e at two updates: 172,
  when the pointer reached Have A Snack, and 223, the update before the
  one the release ended in (224). It did not run for the mousedown on
  the entry (ended 202).
- **Read in the code (not traced).** The pie window's message handler
  (0x5dc9a9) goes through its vtable (from 0x674b54):
  - for a mouse move, the handler at vtable +0xfc tells the house view
    about the entry under the pointer, with 0 (the call at 0x5eb5a9);
  - for a button-up, the handler at +0xf4 (0x5eb433) checks the pie's
    own flags and then tells the house view about the entry under the
    pointer with 1, meaning chosen (the call at 0x5eb48e).

  The house view's UI handler (0x51b7f6) runs the entry check for its
  event 14. That both calls reach it there is what the watch shows; the
  path between the two is not read.
- **Ported.** `Game::press` on an open pie checks the entry under the
  point (`Game::check_entry`, the same check the hover runs) before it
  chooses it. The check's sounds are kept, and the action it leaves is
  the one chosen. If our check stops with an error, its note is kept in
  front of what the choice says.
- **Open: press or release.** The original chooses an entry, and checks
  it, at the button's release; nothing ran at the mousedown. The window
  chooses at the press, as it did before this change, so a press on an
  entry followed by a drag off the pie and a release chooses the entry
  in ours and not, most likely, in the original. The replays give the
  press at the update the release ended in, so they do not show this.
  Choosing at the release would change how the window takes clicks for
  every pie, and is left for a later change.

## What it changes

- **Read.** Mortimer's frame locals at 187 (tree 4116 local 1 = 254,
  tree 4114 local 0 = 285) now match the recording: they are what the
  second check leaves in his frame storage with his frames ending at
  word 68. The Read test pins them. The Read note's open item is
  settled.
- **Cancel.** The release's check accounts for the differences at
  257–259 (his word 50 and last result), which were taken for a listing
  under the still pointer. The write watch that shows the release check
  is the snack-bob recording's; the cancel recording does not rule out
  a listing at the same updates whose effects the check covers. The
  cancel replay now equals the recording in every word the comparison
  checks, after every update from 1 to 1300 (scratch tool). The cancel
  test pins word 50 and the last result at 257.
- **Scripted snack.** The scripted snack order no longer checks its
  entry, so the window replay's lot now differs from it until 205, not
  150: the snack's frames from 150 start with what the second check
  left.
- **Talk, Sit, snack.** No change in what the comparison finds.

## The snack order with plumb-bob shots

The recording has a press on the fridge (118) and Have A Snack. The
test is
`the_recorded_snack_order_with_bob_shots_replayed_in_the_window_matches_the_recording`.

- **When.** Each input is given at the update the watches show it acted
  in:
  - pointer moves at 85 and 91 list nothing. The second move's command
    ran from 90 to 93, and the original listed nothing at the frame
    after 92; the replay moves at 91, before that frame, which the
    recording does not pin to one update;
  - the move onto the fridge at 122 lists its pie (Mortimer's last
    result goes to 0 at 123);
  - the press at 152 lists it again: Have A Snack, Have Quick
    Breakfast, Have Breakfast, Serve Breakfast;
  - the pointer reaches Have A Snack at 172, and the release chooses it
    at 223. Have A Snack (4103 on the fridge) runs at 224 with an empty
    queue, as recorded;
  - after the choice the pointer goes to the recording's (200, 123).
- **Frames.** `0,1,2,3,32,62,92,122,152,172,201,223,253+30`. Every
  person's Room motive takes its new score at 524.
- **Verified.** In every word the comparison checks, the lot equals the
  recording's after every update from 1 to 800 except:
  - Bella's frame of tree 8292, local 0, at 203–227 (recorded 16104,
    ours 0), as in the snack and Read notes;
  - Mortimer's word 50 at 644–800 (recorded 1, ours 0).

## Open: under the still pointer, 643

After 643 the original listed a pie under the still pointer: four
entries were checked, the first leaving word 50 at 0 and the other
three setting it (0x452382). The watch's call chains are cut short, so
they do not name the object. The screenshot taken from 644 matches our
frame after 643: the bob is drawn at about the recording's (213, 145),
13 pixels right of and 22 below the pointer at (200, 123). Trial
listings at 643 (scratch tool) of 17 objects leave word 50 at 1; those
of the bob and of the salesman close the difference. Which object the original listed is
**unverified**. Track 2's pick-watch recording should name it.

The replayer and comparer used to find these are scratch tools, not in
the repository.
