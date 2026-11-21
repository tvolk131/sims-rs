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
  - The top pie's press is given once the lot has run update 123.
    The harness's input log first reports the pie at 124, and the
    saved state shows Mortimer's last result changing at 124. Its button went down in
    a command that ended at 122 and came up in one that ended at 139,
    so this press does not follow the rule above.
  - The Talk... entry is pressed at 176 (its release ended at 177).
  - About Interests is pressed at 250 (its release ended at 251, when
    the talk is queued).
  - After the choice the pointer goes back to the recording's
    (400, 273), and it stays there to the end.
- **Where.** At the recording's points, moved 97 pixels right and 46
  down. The exception is Bella: when our pick misses her at the
  recording's point, both the hover and the press on her use a
  substitute point (below). The pie then opens around that point, and
  its entries keep their places relative to it. Pie entries are found
  by label. The recording's screenshot at 177 shows the same six top
  entries as ours, in the same places. The harness chose the
  submenu's first entry, which is About Interests in ours.
- **Frames.** Every person's Room motive takes its room's new score at
  521, so the original drew a frame after update 520. The replay draws
  one after every 30th update from 190 (the cadence of the Sit
  recording's frames), and the default's frames before that.

## Verified

In every word the comparison checks, the lot equals the recording's
after every update from 1 to 1000 except:

- Mortimer's word 50 at 521–810 (recorded 1, ours 0) and 881–925
  (recorded 0, ours 1);
- local 0 of his fifth frame, tree 8333, at 568–588 (recorded 265, ours 0).

These most likely come from listings the original made under the still
pointer and ours did not. Which objects it listed is **unverified**
(below).

The comparison skips every word our state marks as unknown. As in the
Sit and cancel notes, these include two of the salesman's from update
14:

- his person word 37 (3289 recorded, 0 ours);
- local 7 of his third frame, tree 0x1003 (2625 recorded, 0 ours).

Others' person word 37 is skipped the same way: Alexander's at 588
(3092 recorded, 0 ours) and Bella's at 1000 (2876 recorded, 0 ours).

The test pins:

- Mortimer's last result going from 1 to 0 with the hover that lists
  Bella's pie;
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
    (499, 317). The test uses the recording's point once our pick
    finds her there.
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
  three rows below them. The salesman's nearest pixel is (501, 319),
  4 columns right on the same row.
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
  - A trial listing of the salesman at 550 (scratch tool) puts tree
    265 at word 56, and tree 8333 then starts with local 0 = 265 at
    568, as recorded.
  - So the original most likely listed again in that window. The
    recording shows only the value left behind, not what wrote it.
    That the frame after 550 was its only frame there is also
    **unverified**: the schedule is settled only at 520.
  - The salesman is still nearest the pointer at 550. While the
    pointer is still, ours lists only when the object under it changes
    (`Game::follow`), as the original's house tool does (0x48c834); a
    press lists again. Why the original listed again is not known.
- **Again at 880.** Mortimer's word 50 goes from 1 to 0 at 881, while
  ours stays 1. Every entry looked at clears it, and a check tree may
  set it again, so the original most likely listed something whose
  checks leave it at 0. Listing Bella leaves it at 0. In our frame
  after 880, Bella is drawn 2 columns right and 2 rows up of the
  pointer.
- **Together.** With three trial listings, the replay equals the
  recording in every word the comparison checks, after every update
  from 1 to 1000, with none of the differences above left (scratch
  tool):
  - the salesman after 520;
  - the salesman again after 550;
  - Bella after 880.

  That fits these three listings. It does not show that the original
  made them, or no others whose effects do not show.
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
