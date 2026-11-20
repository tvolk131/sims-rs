# The Talk order replays in the window; the original's person pick reaches past the drawn Sim

*Decided 2026-11-20.*

Track 2's recording `goth-talk-order-20261118` has the original ordering
Mortimer to talk to Bella about interests:

- a press on Bella opens her pie;
- Talk... opens its submenu;
- About Interests queues the talk.

It is now replayed through the window's input layer, as the Sit and
cancel orders are (`20261118-sit-and-cancel-orders-replayed-in-the-window.md`).
The test is `the_recorded_talk_order_replayed_in_the_window_matches_the_recording`.

## How the input is given

- **When.** As for the Sit and cancel orders: each input is given once
  the lot has run the update before the one its command ended in.
  - The hover that lists Bella's pie ended at 94, and Mortimer's last
    result changes at 94.
  - The top pie's press is given at 123. Its button went down in a
    command that ended at 122 and came up in one that ended at 139.
  - The Talk... entry is pressed at 176 (its release ended at 177).
  - About Interests is pressed at 250 (its release ended at 251, when
    the talk is queued).
  - After the choice the pointer goes back to the recording's
    (400, 273), and it stays there to the end.
- **Where.** At the recording's points, moved 97 pixels right and 46
  down, except for the press on Bella (below). Pie entries are found by
  label. The recording's screenshot at 177 shows the same six top
  entries as ours, in the same places. The harness chose the
  submenu's first entry, which is About Interests in ours.
- **Frames.** Every person's Room motive takes its room's new score at
  521, so the original drew a frame after update 520. The replay draws
  one after every 30th update from 190 (the cadence of the Sit
  recording's frames), and the default's frames before that.

## Verified

The lot equals the recording's after every update from 1 to 1000 except:

- Mortimer's word 50 at 521–810 and 881–925 (recorded 1, ours 0);
- local 0 of his fifth frame, tree 8333, at 568–588 (recorded 265, ours 0).

All of these come from listings the original made under the still
pointer and ours did not (below). The test pins:

- the top pie and the submenu's first entry;
- the talk queued behind his reading through the social object (260):
  his action 4235 and Bella's answer 4236 at 251, his running at 252;
- everyone's Room motive at 520 and 521.

## Open: the original's person pick reaches past the drawn Sim

These are **unverified** and not ported.

- **The press on Bella.** The original found Bella at the recording's
  (400, 274), our (497, 320).
  - Her lowest pixel in our frame after 93 is (499, 317), three rows
    above.
  - Our pick finds nothing at (497, 320), so the replay points at
    (499, 317).
  - The recording's screenshot at 60–64 puts her feet on the same
    rows as our frame after 63, within a row.
- **Under the still pointer, 520.** Trial listings at 520 (scratch tool)
  of the salesman (259), the social object (260), Bella, Mortimer,
  Alexander (16), the door (51), the bookcase (132), the benches (173,
  174) and 171 leave this state:
  - Only the salesman's leaves Mortimer's last result 1 and his word
    50 at 1, as the recording shows from 521.
  - Listing him at 520 removes the word-50 difference at 521–810.

  So the original most likely listed the salesman. In our frame after
  520, the pointer's pixel is empty: it falls between Bella's legs,
  three rows below them. The salesman's nearest pixel is 4 columns
  right and 4 rows up.
- **Again between 543 and 567.** Tree 8333's local 0 is not written
  before it is read: it starts as what an earlier frame left in
  Mortimer's frame storage. Its value is 265 in the recording.
  - From 543 his frames end at word 48. A listing then writes the
    original's mark at words 48–55, before running a check tree
    synchronously (0x4749b5). The check tree's frame starts at word 56,
    and the first word of a frame is its tree's ID.
  - Listing the salesman runs global tree 265 there. In ours, the
    listing at 520 wrote it at word 64, above the mark over his
    then-top frame (8300).
  - So the original listed again in that window. The frame after 550
    is the only frame in it, and the salesman is still nearest the
    pointer then. Ours lists only when the object under the pointer
    changes, as the original's house tool does (0x48c834). Why the
    original listed again is not known.
- **Again at 880.** Mortimer's word 50 goes to 1 at 881. In our frame
  after 880, Bella is drawn 2 columns right and 2 rows up of the
  pointer.
- **What the original does.** A person's draw (0x590ebd to 0x590f84) takes the
  cursor from GetCursorPos and ScreenToClient (the window at the view's
  +0x2c, +0x128). It reads the 16-bit depth there (0x4a973a) before and
  after drawing the person (0x5a1602), and notes the person when the
  two differ. So the pick follows what the draw writes to depth, which
  may cover pixels the colour does not. The screenshot shows nothing
  drawn below Bella's feet, not even a shadow.

  A fixed offset between that cursor and the message's point does not
  explain the cases. No shift within 10 pixels puts both Bella at 93
  and the salesman at 520 under the pointer.

These listings are the "under the still pointer" case of
`20261119-pointer-pick-is-the-originals-sprite-pick.md`.

## What would settle it

On this recording's scenario, at updates 85–100, 515–560 and 875–885:

- a write watch on the noted graphic (0x6f6bc4);
- a write watch on the house tool's current-object word (+0x2e of
  0x48c834's `this`, written at 0x48c8d9);
- screenshots after 520, 550 and 880.

The replayer and comparer used to find these are scratch tools, not in
the repository.
