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
  - The register write follows the track's `target` (0x57de57). Here
    it goes to the thread the track's last start made, which stands for
    the original's one track object (see below), and on through its
    `target`. A fade in progress goes on from the new volume.
  - The write's `target` chain reaches a thread whose script has ended
    while its notes play on, as the original's pointer reaches a track
    object that outlives its script, and those notes take the write at
    once. A link to a thread with nothing left playing still stops the
    chain (**unverified**, as before).
  - The last-start rule below holds at every link: a thread on the way
    that is not its track's last start takes the write itself, so a
    `target` it set before its track started again carries nothing on.
  - Only the camera's write follows ended threads. The scripts' own
    register accesses through `target` stop at a thread that has ended,
    as before. Following them there too would let a stop's fade or a
    script's level write reach another track's lingering note, which is
    not known to happen.
  - The original's track object outlives its script. A note started
    with `note_on` then `end` plays on, and the write reaches that note
    too. Here a thread that ends with notes still playing keeps its
    registers while they play. The write also goes to every such thread
    of the track, and their notes take the new levels at once.
  - Only the thread of the track's last start, running or ended,
    writes and reads through its `target`. A later start set the
    original's track object again, with its `target` back to itself, so
    an older thread takes the write itself: an ended one, or the newest
    still running when the last start's thread has gone. Otherwise a
    `target` that thread had set would carry the write to another
    track. The player keeps which thread each track's last start made,
    so this holds after that thread has gone with nothing left playing.
  - Every start sets the track object's `target` back to itself
    (0x57d9c3), before it checks for a hold. So a start that lets a
    held track go, rather than starting it again, counts too: the
    newest thread it lets go becomes the one written through. Unpausing
    lets every track go without that reset (0x57dc63).
  - A note kind 53 played with no thread of the track running (thread
    0, the original's idle track object) is placed too, from the
    registers it started with, kept while it plays.
  - `Spl` comes from the thread of the track's last start, or else
    from the newest running thread, the newest ended one or the kind 53
    note's registers, in that order.
  - A track with none of these gets nothing. The original writes to its
    idle track object, and the next start sets those registers again
    anyway.
  - **Unverified:** the original's older notes follow its one track
    object, so they would take the newest start's levels. Here each
    keeps its own thread's, as notes do elsewhere in the player.
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
  new place, and so does a kind 53 note played with no thread running.
- A thread whose track started again does not carry the write through
  the `target` it had set, whether it ended with a note playing while
  the new one waits or ends at once, or it still runs while the new one
  ended with a note playing, or the new start let a held thread go: the
  other track keeps its levels.
- A write through a `target` whose script has ended reaches its note
  still playing: placed by another track's object at the right edge,
  it turns right. Through two links it reaches the track after, unless
  the middle track has started again since; then its old thread takes
  the write and the last track keeps the centre.
- A stop's fade stays with the track it stops, though that track's
  `target` names one whose script has ended with a note playing.
- A track whose thread has ended and left no note gets nothing, and its
  next start is placed afresh.
- The Goths' lot plays with the camera scrolling every second.
- A change of walls sends nothing.

**What would reopen it:** reading the five senders, or a capture of the
original that shows when it sends kind 33 or kind 25.
