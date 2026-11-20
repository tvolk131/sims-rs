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
  0x57d960), then sets its volume and pan (+0x40, +0x48), then lets it go
  (+0x5c). `play_trk` (0x57f1f2) calls +0x2c (0x57dc0d), which is the start
  followed by the let-go.
- Every start begins the same way (0x57d9b2 to 0x57d9eb). It clears
  `fade_on` (+0xa8) and the flag that ends the track when its fade ends
  (+0x1cf). It points `target` back at the track (0x57d9c3) and writes the
  arguments to registers 1 to 3.
- If the track is running (+0x1c4) and the pause holds it, the start lets it
  go and returns (0x57d9f1 to 0x57da12). This is unchanged.
- If the track is running and not held, the start reads register 55,
  `interrupt` (0x57da26 to 0x57da34):
  - 0 returns, so the script goes on where it was.
  - Any other value takes the fresh-start path. That path resets the
    registers from the default banks, sets the patch through 0x57f625, and
    0x57f625 destroys the track's voice (0x57f657 to 0x57f676), so the note
    is cut. The script then plays from the top.
- Letting a track go (+0x5c, 0x57dc63) moves its wait deadline to now
  (0x57dc8e), so a running `wait` ends at the next update.
- Only `loadb` writes `interrupt`. In the install, 99 of the 4,306 tracks
  write it.
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
  track's threads (`set_arguments` now clears the fade too) and starts no
  new one unless `interrupt` is set. In that case it ends the track,
  cutting its notes, and starts it fresh.
- Its callers let the threads go (`Player::let_go`) as the original does:
  - kind 1, `play_trk` and the piano always;
  - kind 30 only when the sound is not paused.
- Kind 30 now holds a track during a pause only when the start was fresh.
- A fresh start stops the note that kind 53 played on the idle track
  (0x57f625).
- When `play_trk` starts the track that is running it, the start acts on
  that running thread. A fresh start there ends the thread, and the new
  thread runs in the next block.
- An object going leaves a track that others still have playing, placed
  again by the camera move's per-track placement, now
  `Player::place_track`. Before, that object's own thread faded out.
- The thread limit applies only to fresh starts.
- The Goth sound test runs to the end of the recording again.

## Not ported, and unverified

- The original's run loop (0x57d8b3) runs instructions while the wait
  deadline is set and past. The fade-out-and-kill does not move the
  deadline. So, by the code, a script that ends with its note still playing
  goes on to the bytes after its `end` while the note fades. Here the
  thread ends and the note plays on.
  - The track then counts as not running, and a later start makes a new
    thread beside the note.
  - In the original, the track is still running, and a start would only
    refresh it.
  - Not ported. Whether the original really runs past `end` is
    **unverified** at run time.
- Ducking on a restart is **unverified**. The ended thread's
  `main_duckpri` request goes when it ends. The original's one track
  object may keep its request across the restart.
- `play_trk` restarting the track that runs it is a guess, marked
  **unverified**. The original goes on from the top within the same update.
  Here the new thread waits for the next block, as every thread `play_trk`
  starts does.
