# The Read A Book order replays in the window; Mortimer's stale locals fit a second check of the entry

*Decided 2026-11-20.*

Track 2's recording `goth-read-order-20261118` has the original ordering
Mortimer to read a book:

- a press on the bookcase (132) opens its pie;
- Read a Book is chosen, and he starts reading at once.

It is now replayed through the window's input layer, as the Sit, cancel
and Talk orders are (`20261118-sit-and-cancel-orders-replayed-in-the-window.md`,
`20261120-talk-order-replayed-in-the-window.md`). The test is
`the_recorded_read_order_replayed_in_the_window_matches_the_recording`.

## How the input is given

- **When.** As for the earlier orders: each input is given once the lot
  has run the update before the one its command ended in.
  - The first move ended at 88, so the replay points at the
    recording's (384, 262) at 87. Nothing is listed: the harness
    reports no object there at 92.
  - The second move ended at 94, so the replay points at the bookcase
    at 93. Mortimer's last result is 1 in the recording from 84 to 186,
    and stays 1 in ours with this listing.
  - The press on the bookcase is given once the lot has run update
    123. Its button went down in a command that ended at 122 and came
    up in one that ended at 144, and the harness first reports the pie
    at 124. This follows the Talk order's top pie, not the rule above.
  - The move onto Read a Book ended at 144, so the replay reaches the
    entry at 143.
  - On the entry, the button went down in a command that ended at 163
    and came up in one that ended at 187. The harness reports the
    order queued and taken at 187. The replay presses the entry at
    186, as the earlier orders press an entry at their release.
  - After the choice the pointer goes back to the recording's
    (384, 324), and it stays there to the end.
- **Where.** At the recording's points, moved 97 pixels right and 46
  down. The exception is the bookcase (below). The pie then opens
  around the replay's point, and the entry is found by label. The
  harness saw the same three entries as ours: Study Cooking, Study
  Mechanical, Read a Book.
- **Frames.** Every person's Room motive takes its room's new score at
  607, so the original drew a frame after update 606. The replay draws
  one after every 30th update from 186 (606 is the fourteenth), and the
  default's frames before that.

## Verified

In every word the comparison checks, the lot equals the recording's
after every update from 1 to 900 except:

- Mortimer's frame of tree 4116, local 1, at 187–370 (recorded 254,
  ours 0);
- his frame of tree 4114, local 0, at 187–321 (recorded 285, ours
  4123). These two are gone since the second check of a chosen entry
  was ported (below);
- Bella's frame of tree 8292, local 0, at 203–227 (recorded 15837,
  ours 0), as in the snack and queue notes
  (`20261115-press-lists-pie-menu-again-reaching-entry-checks.md`,
  `20261115-replay-draws-recording-frames-queue-listing.md`);
- Bella's frame of tree 8229, locals 2 and 3, at 368–378 (recorded 768
  and 15873, ours 0);
- Mortimer's word 50 at 727–900 (recorded 1, ours 0), and his last
  result at 727–874 (recorded 1, ours 0).

The comparison skips every word our state marks as unknown, as in the
Talk note. These include the salesman's (259) person word 37 (3090
recorded from 14) and local 7 of his frame of tree 0x1003 (2609
recorded at 14).

The test pins:

- the bookcase's pie listed by the hover at 93, with Mortimer's last
  result 1;
- the pie's three entries;
- no running action before the choice, then Read a Book (4110 on the
  bookcase) running at 187 with an empty queue and his last result 0,
  as recorded;
- everyone's Room motive at 606 and 607;
- nothing caught to 900.

## Settled: the entry is checked a second time before the order starts

Choosing the entry checks it again: the snack recording with plumb-bob
shots shows the check run at the release, and it is now ported
(`20261120-choosing-a-pie-entry-checks-it-again.md`). Mortimer's two
frame differences below are gone, and the test pins those locals. What
follows is the reasoning as it stood before.

- **What the words show.** New frame locals are not cleared: they
  start as what earlier frames left at that place in the Sim's frame
  storage. Reaching Read a Book runs its check tree (4123), which runs
  global tree 285. A synchronous check-tree run first writes an
  8-word mark at the end of the frames, and the check tree's frame
  follows it. The first word of a frame is its tree's ID and the
  second its node.
  - Ours checks the entry once, at 143. Mortimer's frames then end at
    word 80: the mark is at 80–87, tree 4123's frame at 88 and tree
    285's at 100.
  - From 150 his frames end at word 68, in the recording and in ours:
    the frame of tree 8292 that was at 68 has ended. With his frames
    ending there, the same check puts tree 4123's frame at 76,
    its node (254) at 77, and tree 285's frame at 88.
  - At 187, tree 4116's frame starts at 68 with local 1 at word 77, and
    tree 4114's frame has local 0 at word 88. The recording has 254
    and 285 there, first at 187: what the check leaves with his frames
    ending at 68. A second check after 150 is a plausible explanation.
    The recording has no trace of the check itself, so what wrote
    those words, when, and what made it run are not shown.
- **Trials (scratch tool).** Leaving the entry and coming back onto it,
  which checks it again, at 186 before the press removes both of
  Mortimer's frame differences. Doing it at 162 instead does too. No
  single trial listing at 186 of the 330 objects ours has (the
  recording has 345) leaves 285 at word 88: 284 leave 4123, 22 leave
  265, 14 leave 0 and 10 leave 321.
- **In the original (from reading the code, not traced).** A UI event
  handler (0x51b7f6) runs the entry's check (0x48df28, called at
  0x51bb9e) for its event 14. Event 15 calls 0x48dd67. Where the pie
  sends event 14, and whether a mousedown or a release on an entry
  sends it again, is not known.

## Open: under the still pointer, 726

Mortimer's word 50 goes from 0 to 1 at 727, and his last result from 0
to 1, while ours stay 0 (his last result to 874). A trial listing of the salesman at 726
(scratch tool) leaves both at 1, as recorded. In our frame after 726,
the pointer's pixel is lawn. The salesman's nearest pixel is 11
columns right and 11 rows up. This is another case of "under the still
pointer" (`20261119-pointer-pick-is-the-originals-sprite-pick.md`). Which
object the original listed is **unverified**.

## Open: the bookcase's point

The original found the bookcase at the recording's (384, 250), our
(481, 296). In our frame after 93 that pixel is wall, where our sprite
pick finds the door (51). So the replay points at (488, 296), 7 columns right on
the same row, where our pick finds the bookcase. The test uses the
recording's point once our pick finds the bookcase there. This is the
"salesman vs bookcase" case.

## Bella's stale locals

- Tree 8292's local 0 at 203–227 differs as in the snack recording;
  this recording has 15837 where that one had 16104.
- Tree 8229's locals 2 and 3 at 368–378 are 0 in ours and 768 and
  15873 in the recording. What left those values is not known. One
  possibility is storage that no frame since the load has written:
  ours starts that at zero, and what the original's holds there is
  **unverified**.

## Together

With the second check of the entry at 186 and a salesman listing at
726 (scratch tool), the replay equals the recording in every word the
comparison checks, after every update from 1 to 900, except Bella's
three stale locals in two frames. That fits; it does not show that the original did
these things, or nothing else whose effects do not show.

## What would settle it

On this recording's scenario:

- an execute watch on the entry check's call (0x51bb9e) at updates
  140–190, giving each update it ran in;
- a write watch on the noted graphic (0x6f6bc4) and the house tool's
  current-object word (+0x2e of 0x48c834's `this`, written at
  0x48c8d9) at updates 85–100 and 720–730;
- screenshots after 93 and 726.

The replayer and comparer used to find these are scratch tools, not in
the repository.
