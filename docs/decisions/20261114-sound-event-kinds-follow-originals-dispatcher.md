# Sound event kinds follow the original's dispatcher

*Decided 2026-11-14.*

The player now acts on an event by its kind, as the original's event
dispatcher does (read from its code in a memory image, never written).
Each event gives a track and three arguments; an argument the event
leaves at 0 takes the caller's value, which is the object for the
first. So a fade event, which never gives a speed in the install, fades
at the object's number, or at 100 for no object. That is what the code
does, odd as it looks.

Ported: play (1), fade (18), `radio_next_song` (19), kill all (20),
kill an object's instances (23), turn on a television or radio (30),
sequence a hit list (53) and kill every object's sounds (105). Kinds
whose case does nothing for sound are ignored. Not ported, and counted
in a gated test: music modes (36, 21 events), ambience modes (52, 5
events, the ambient scores), the piano (43), pausing (103) and
reloading (104). Twelve play events have an empty track and do nothing.

The radio, television and music hit lists come from directories, as the
original builds them when it starts: one sub list per subdirectory, one
patch per file. Their numbers are made up (from 2^30), since the
original's come from a counter. Files are taken in sorted order; the
original takes them as the file system lists them.

`SoundEvent::StopObject` is now kind 23: tracks the object was recorded
as playing for fade out at speed 100 and end, or end at once when
silent. Threads the object started without being recorded (tracks whose
arguments are not `kArgsIdVolPan`) keep running, as in the original.

MP3s are resampled to 22,050 Hz when loaded. The original sets every
sound buffer's rate from `pitch` (22,050 Hz at 3600), and its MP3s are
44.1 kHz, so it must hand DirectSound 22,050 Hz from its MP3 data
source, or play songs an octave low. Which it does is **unverified**.

Model limits: the original has one track object per track number,
shared by everyone playing it; this player runs a thread per play and
uses a track's newest thread where the original uses its one object.
When an object lets go of a track others still have, the original
re-evaluates the track (0x5746d9, not read); here that object's threads
fade out. A note whose thread has ended has no registers left to fade,
so it stops at once when its track is let go.

Reopen when a run with sound can be compared, when the music modes are
ported, or when the simulation needs pause.
