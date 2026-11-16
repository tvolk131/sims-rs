# The piano plays where its object is, and a start sets a track's levels

*Decided 2026-11-15.*

Kind 43 (`kPlayPiano`) is ported from the sound manager's case for it
(0x573e5a). The event's own track is not used: the piano is track 1906
(`Piano`, in `SimsGeneratedHitSource`), and nothing happens without it.

- **The piano's start:** the object (the first argument) is placed as
  one object with the piano's `Spl` (0x5730b6, as kinds 1 and 30 place
  each of their objects), into the second and third arguments. When it
  is not placed they keep the event's values. The piano then starts and
  is let go (vtable +0x2c), as kind 1 starts a track.
- **Its instances:** all of the piano's recorded objects are dropped
  (0x581f8e on the map at +0xd4), and the track object's own object
  (vtable +0x24, the field at +0xb0) is recorded. Only a fresh start of
  a `kArgsIdVolPan` track sets that field (vtable +0x20, 0x57db2e). So
  a start that lets a held piano go, or one refused at the thread
  limit, leaves the object of the last fresh start, and before any it
  is 0. The player now keeps that field for every track.
- **The skill:** the object's source field 5 is asked for (0x570e61),
  before track 1907 (`SetPianoSkill`) is looked up. When that track
  exists it starts with the skill as its first argument. Its script
  writes it into the piano's register 5.
- **A start sets the track's levels** (0x57db01): a fresh start writes
  `extvol` and `pan` from the arguments, as the track's `args` field
  says: the first two for `kArgsVolPan` (the stations), the second and
  third for `kArgsIdVolPan`. The player left them at their defaults.
  Kinds 1 and 30 place the track after its start: an object that cannot
  be placed still takes the sum's values (see the entry on placement),
  so only a placement fault now leaves them the event's levels. `play_trk`
  now hands a started `kArgsIdVolPan` track the caller's registers 2
  and 3 as its levels. A held track let go by a start, and the idle
  track kind 53 plays on, keep theirs, as in the original.
- **Model limit:** the original has one piano track object, so a second
  piano event restarts it; here it runs a second thread (as for every
  track; see the entry on the sound player's model).
- **Tests:** hand-built modules check:
  - the placement, and the event's levels kept;
  - the skill asked for, with and without its track, and its start;
  - the instances replaced, also when the start lets a held piano go or
    is refused at the thread limit;
  - the levels a fresh start sets for `kArgsVolPan` and for a
    `kArgsIdVolPan` track that `play_trk` starts. On the
  install every event still runs without a bad instruction, and
  `piano_play` now plays notes for ever.
