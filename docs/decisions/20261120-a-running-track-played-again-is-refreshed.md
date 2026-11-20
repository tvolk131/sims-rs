# A running track played again is refreshed, not started twice

*Decided 2026-11-20.*

In the Goth sound test, the fountain played its track every 300 updates, and
each play started another thread that never ended. Around update 74,700 the
player hit its 256-thread limit, so #273 capped the test at 43,200 updates.
The question was what the original does when a track that is still playing
is played again. The answer is in its sound code: one track object per track
number, and a start of a running one does not start a second voice.

## The original

- The original keeps one track object per track number (looked up at
  0x573312). Kind 1 (0x5736ee) calls the object's start (vtable +0x30,
  0x57d960). For a track whose arguments are `kArgsIdVolPan` only, it then
  sets the volume and pan (+0x40, +0x48; 0x573756, 0x5737b3). It always
  lets the track go after (+0x5c). `play_trk` (0x57f1f2) calls +0x2c
  (0x57dc0d), which is the start followed by the let-go. The piano's
  starts and the music modes' starts use +0x2c too (piano 0x573e95,
  0x573ef2; music 0x57523f, 0x5752a4, 0x57540e, 0x575501, 0x5755f1).
- When the track's group is off (manager lookup 0x57bde7, its +0xc0 is 0),
  the start returns before anything else (0x57d969 to 0x57d9a5).
- Every other start begins the same way (0x57d9b2 to 0x57d9eb). It clears
  `fade_on` (+0xa8) and the flag that ends the track when its fade ends
  (+0x1cf). It points `target` back at the track (0x57d9c3) and writes the
  arguments to registers 1 to 3.
- If the track is running (+0x1c4) and the pause holds it, the start lets it
  go and returns (0x57d9f1 to 0x57da12). This is unchanged.
- If the track is running and not held, the start reads register 55,
  `interrupt` (0x57da26 to 0x57da34):
  - 0 returns, so the script goes on where it was.
  - Any other value takes the fresh-start path on the same track object.
    That path resets registers 17 to 47 and 50 to 62 from their defaults
    (0x57d727 to 0x57d741); registers 1 to 16 and the return stack
    (+0x1a8) are kept. (Registers 48 and 49 are in no bank of the
    original, 0x57d727 to 0x57d775 and 0x57c7a8 to 0x57c82e.) It sets the
    program counter and the loop point to the entry from the track
    object's own descriptor (0x57db7a to 0x57db91), whichever module the
    request came from, and resets the hit list
    (0x57db85 to 0x57dba7). It sets the patch through 0x57f625, which
    destroys the track's voice (0x57f657 to 0x57f676), so the note is cut.
    It sets the deadline to now, `isplaying` to 1 (0x57dbc5) and running
    to 1 (0x57dbcf). Anything pointing `target` at the track still
    reaches it.
- Letting a track go (+0x5c, 0x57dc63) moves its wait deadline to now
  (0x57dc8e), so a running `wait` ends at the next update. A running track
  let go also moves to the end of the list of running tracks (0x57dc9a:
  0x57d017 takes it out and appends it), so from the next update it runs
  after the others. Kind 30 lets go only when the sound is not paused
  (0x573d71).
- `play_trk` reads `source` (register 47) after the start (0x57f256), so a
  track that starts itself hands on its own `source`: the start has pointed
  its `target` back at itself.
- The update loop takes a snapshot of the running tracks before it runs
  them, and calls each track object in it (0x57cf8f to 0x57cfda). A track
  ended or started during the update does not move another out of its
  turn. A track ended (the kill clears its running flag, 0x57dd4c) and
  started again (0x57dbcf) before its turn is the same object, so it still
  runs in that turn.
- A fade that was to end its track ends it at once when it reaches its
  end (0x57d86e to 0x57d88d).
- In the install only `loadb` writes `interrupt`: 99 of the 4,306 tracks.
  `set` can write it too (0x57e219 to 0x57e2da).
- The fountain's track 1346 is `note_on; wait_samp; loop`. It never sets
  `interrupt`, so the original ignores every play after the first: it only
  refreshes the arguments and places the track again.
- Kind 23 calls 0x574688 for each track that an object going had. If no
  object is left, the track fades out and ends (+0x34). If others still
  have it, 0x5746d9 places it again from where they are: 0x57473b, then
  +0x40 and +0x48. That is the same per-track placement as the camera
  move's.
- The running flag is cleared only by the kill (+0x38, 0x57dd3e). A
  script's `end` with nothing to return to (0x57e3ee to 0x57edc8) calls the
  fade-out-and-kill (+0x34, 0x57dca3). With a note still playing, that
  fades the note at speed 100 and keeps the track running until the fade
  ends. With no note playing, it kills the track at once.

## What changes here

- A track runs one thread at most. `Player::start` refreshes a running
  track's thread (`set_arguments` now clears the fade too) and starts no
  new one unless `interrupt` is set.
- With `interrupt` set, `Player::restart` starts the same thread anew, in
  place, from the module the thread came from. It keeps its id, so
  `target`s that point at it still reach it. It resets only the default
  banks (registers 17 to 47 and 50 to 62, `BANKS`), keeping registers 1
  to 16 and the return stack (and 48 and 49, which the original has no
  bank for). It sets the program counter and loop point to the entry,
  clears the hit list, and cuts the track's notes.
- Its callers let the threads go (`Player::let_go`) as the original does:
  - kind 1, `play_trk`, the piano and the music modes always;
  - kind 30 only when the sound is not paused.
- Kind 30 now holds a track during a pause only when the start was fresh.
- A fresh start stops the note that kind 53 played on the idle track
  (0x57f625).
- When `play_trk` starts the track that is running it, the start acts on
  that running thread. A restart there runs it from the top in the next
  block.
- `play_trk` reads `source` after the start.
- Each thread has a place in the list of running tracks (`Thread::order`);
  a fresh start and a let-go put it last. `Player::run` takes the list of
  tracks in that order at the start of a block, and at each track's turn
  runs the thread it has then. A thread ended or restarted during the
  block does not make another skip its turn, and a track ended and
  started again before its turn runs in it.
- When the block runs out of instructions, the tracks not reached still
  fade, and one whose fade was to end it ends then.
- An object going leaves a track that others still have playing, placed
  again by the camera move's per-track placement, now
  `Player::place_track`. Before, that object's own thread faded out.
- The thread limit applies only to fresh starts.
- The Goth sound test runs to the end of the recording again.

## Not ported, and unverified

- The group-off return (0x57d969 to 0x57d9a5). This player has no group
  that can be off.
- The original's run loop (0x57d8b3) runs instructions while the wait
  deadline is set and past. The fade-out-and-kill does not move the
  deadline. So, by the code, a script that ends with its note still playing
  goes on to the bytes after its `end` while the note fades. Here the
  thread ends and the note plays on.
  - The track then counts as not running, and a later start makes a new
    thread beside the note.
  - In the original, the track is still running. A start during the fade
    still reads `interrupt`, and clears the fade.
  - The new thread here starts with registers 1 to 16 at their
    defaults. The original's track object would keep them from the run
    before. This is **unverified**.
  - Not ported. Whether the original really runs past `end` is
    **unverified** at run time.
- Ducking on a restart is **unverified**. The original's one track object
  may keep its `main_duckpri` request across the restart.
- `play_trk` restarting the track that runs it is **unverified** in its
  timing. The original goes on from the top within the same update. Here
  the restarted thread runs from the top in the next block.
