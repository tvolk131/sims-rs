# A camera move places every sound track again (kind 33)

*Decided 2026-11-15.*

The original's sound manager places every track again when its game
sends kind 33 (0x574157). The port used to leave this out: the game
here moved the ambience's cursor once a frame, and placing every track
once a frame would undo a fading `extvol` each frame. Now the game sends
kind 33 only when the camera moves, and the player does the whole case.

- **What kind 33 does** (0x574157):
  - It places every track again (0x57447f).
  - It moves the ambience's cursor from the view and the hour, as
    before.
  - It picks the day and night loop (0x574978), which is still not
    ported.
- **Placing again** (0x57447f, 0x5746d9):
  - The tracks go one at a time, in the instance map's order. For each,
    the volume and pan come from the same sum kind 1 uses (0x57473b).
  - The results are written as the track writes them: `extvol` then
    `pan` (vtable +0x40 and +0x48, 0x57d57a and 0x57d5a0).
  - The register write follows the track's `target` (0x57de57), so it
    goes to the track's newest running thread. That thread stands for
    the original's one track object, as kind 18's fade does. A fade in
    progress goes on from the new volume.
  - The original's track object outlives its script. A note started
    with `note_on` then `end` plays on, and the write reaches that note
    too. Here a thread that ends with notes still playing keeps its
    registers while they play. The write also goes to every such thread
    of the track, through its `target`, and their notes take the new
    levels at once. When no thread of the track runs, `Spl` comes from
    the newest of these ended threads.
  - A track with neither a running thread nor such a note gets nothing.
    The original writes to its idle track object, and the next start
    sets those registers again anyway.
- **A quirk not ported:**
  - After a kind 25 that turns the sound on (0x573c64), the manager's
    +0x114 is 100 for good. Kind 33 then writes the volume less 1, or 1
    for a volume of 0 (0x574701).
  - Kind 25 is not ported, and the flag starts at 0 (0x571433).
  - The original's game sends kind 25 from 0x4f76d5. Whether it does
    so at startup, which would put the less-1 into every camera move,
    is **unverified**.
- **When the game sends it:**
  - The original sends kind 33 from five places in its view code
    (0x4a80bb, 0x4a8d38, 0x4ce50a, 0x4ced58, 0x4f66c6, through 0x57a9d4
    and 0x57aac5). These include the floor changes and the view's
    redraw when its position flags are set. Not all five are read.
  - Here `sims_game` sends it when a `sound::Shot` changes: the view's
    position, zoom, turn, floor or the screen's size. The walls and
    the cutaway's focus are not part of a Shot, because they leave
    every object where it is on the screen.
  - That these are the moves that send it is **unverified**.
- **The ambience's cursor** still follows the hour and the view once a
  frame, so the ambience keeps to the clock while the camera is still.
  As far as is read, the original moves it only on kind 33. Whether
  anything sends kind 33 when the hour changes is **unverified**.
- **Kind 33 as a sound event** is still reported as not ported: no event
  in the install has it, and an event carries no camera.

**Tests:**
- After a camera move, a track started in the middle of the screen is
  at the volume and pan of the object's new place (750 and 542 at the
  right edge). Without a move, it stays where it started.
- A note that plays on after its script ended moves to the object's
  new place.
- A track whose thread has ended and left no note gets nothing, and its
  next start is placed afresh.
- The Goths' lot plays with the camera scrolling every second.
- A change of walls sends nothing.

**What would reopen it:** reading the five senders, or a capture of the
original that shows when it sends kind 33 or kind 25.
