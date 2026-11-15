# Pausing holds the sounds playing, as the original's sound manager does

*Decided 2026-11-15.*

The sound's pause and unpause, kinds 21 and 22, are ported from the
original's dispatcher (0x573c20, 0x573c44), as `SoundEvent::Pause` and
`SoundEvent::Unpause` and as HOT events of those kinds. The game sends them
from its mode handler (0x4ce470, through 0x57a7f2 and 0x57a8e3): pause when
it leaves live mode or pauses in it, unpause when it is back in live mode
and not paused.

A pause (0x57481e) marks every track that is running held (each track's
+0x1c8, through vtable +0x58, 0x57dc2e), except five: `ui_click`,
`MainMusicLoop` and the three speed buttons' sounds (0x5748a2). A held
track's update (0x57d7cd) does nothing, so its script waits and does not
fade, and its buffer stops where it is. Here its threads are skipped and
its notes neither advance nor sound. The pause also writes 0 to
`main_duckpri` (0x57d361), which here drops every thread's ducking
request. In music mode 0 the current ambience player is paused too
(vtable +0x7c, 0x5773af): its update returns at once (0x577af9) until it
is unpaused, while its cells' sounds play on. Starting a track
(0x57d960) clears its hold, so a sound started while paused plays (the
buy mode's clicks do); a held track whose script still runs is not
started again but let go, as an unpause lets it go (0x57d9f1 to
0x57da12), and needs no free thread. Before that check, the start
points the track's `target` back at itself (0x57d9c3) and writes its
argument registers 1 to 3 (0x57d9cb to 0x57d9eb) whatever its argument
kind, so a resumed script sees the new arguments; a new thread here
now gets all three too, where it had only `arg1`, and only for
`kArgsIdVolPan` tracks. A fresh start then writes `isplaying` (register
46) 1 (0x57dbc5), so its script begins with positive comparison flags
whatever its third argument, and ends by holding the track (0x57dbd6).
Kind 1 always lets it go; kind 30 (0x573d14) lets it go only when the
sound is not paused (0x573d71), so a television or radio turned on
during a pause is silent until the unpause. An unpause (0x574904) lets every track go
(0x57cd2c, vtable +0x5c on each) and unpauses the current ambience
player, whatever the music mode. Letting a track go also sets its wait
deadline (+0x198, which `wait` sets at 0x57e453 and the update reads at
0x57d8b3) to the manager's time, so every running script's wait ends at
the next update: a thread waiting a second when the game pauses goes on
as soon as it is unpaused, and an unpause without a pause cuts every
wait short too. Here every thread's wake is set to now.

Not ported: the day and night loop (0x574978), which the pause stops and
the unpause restarts, and the music the unpause restarts (0x575155); this
player has neither. Where the original keeps one object per track, a
track here can have several threads, and the hold is per track, so
starting a held track lets every thread of it go on.

**Unverified:** nothing of this has been compared with the running game.
Reopen when a capture with sound can be compared.
