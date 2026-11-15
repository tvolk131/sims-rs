# The music modes start and stop the music tracks

*Decided 2026-11-15.*

Kind 36's music half is ported from the sound manager's mode change
(0x575170), so the game's modes now play their music. Its ambience half
was already ported. The music half runs only when the mode changes,
and only once the neighbourhood (mode 3) has been shown, as in the
original. The ambience half runs on every kind 36 event, the same mode
again included: after a stop, a second live (0) event starts the live
ambience again though the music does not change.

- **What each mode does** is listed on `Player::change_music`. Each
  start is a track start as kind 1 makes it (vtable +0x2c), with the hit
  list as the first argument; the scripts (`SetMainMusicSongAndHitList`,
  `MainMusicLoop`, ...) do the rest.
- **The expansions in** are a byte of the configuration
  (`[0x6e48bc]+0x66`), which picks the credits' list and whether the
  neighbourhood stops the Grim Reaper's loop. It is taken as every
  expansion (0xff), as the station lists already are; the Complete
  Collection has them all.
- **`play_trk`** (0x57f1f2) now starts the track with the caller's own
  argument registers (1 to 3), not zeros, and copies the caller's
  `source` (register 47), read through the caller's `target` as any
  register read is (vtable +0x88), into the new thread. `MainMusicLoop` reads the
  list its starter was given this way.
- **`kill_trk`** (0x57f284) now ends the whole track, whatever object
  it plays for. It ended only the caller's object's threads, which was a
  guess; the original finds one track object by number and kills it.
- **Not ported:** the 100 ms `Sleep` the original's game thread takes
  around most of these starts, which lets its sound thread run between
  them. Here they happen at once, in order. What that changes is
  **unverified**.
- **Known difference:** the original's sound manager steps its tracks
  and fades once a millisecond (0x57cf37); the player steps them once a
  block (64 frames, about 2.9 ms). The slow fade into live mode is
  therefore about three times slower here. This is left for its own
  change.
- **Tests:** a hand-built module checks each mode's starts and stops,
  and `play_trk`'s and `kill_trk`'s. On the install, every event still
  runs without a bad instruction. 5 more of them leave music or a river
  playing, and the backyard's ambience test now also plays the
  neighbourhood's music, which shares the player's random draws.
