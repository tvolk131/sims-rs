# A track runs from the module that registered it last

*Decided 2026-11-20.*

When two modules define the same track or patch ID, the player used the
requesting module's definition, or else the first module's by name. The
original keeps one object per ID, so every request gets the definition
that registered last. This port now does the same.

## The original

- Each `[Track]` and `[Patch]` line registers an object in the sound
  manager's registry (0x57cdd5; see
  `20261120-unpause-follows-the-sound-registry.md` for the order). An
  ID registered again destroys the old object and keeps the new one in
  its node (0x57ce04 to 0x57ce32).
- A start looks the track up by number (0x573312) and runs from that
  object's own descriptor (0x57db7a to 0x57db91), whichever module's
  event asked. A kill clears the object's running flag (0x57dd4c) but
  keeps the object, so a replay after a kill runs the same descriptor.

## What changes here

- `Hot::registered` records whether each line is a patch or a track.
- The player gives each track and each patch the module of its last
  registration, in the registry's order: `SoundData`, then the
  expansion directories in the original's order, then the downloads.
  (The station patches have their own numbers.)
- Every start, restart and argument kind (`kArgsVolPan`,
  `kArgsIdVolPan`) uses that module. The fallback to the requesting
  module and the running thread's module are gone.
- Events still come from the first module that defines them. This
  change does not touch event lookup.

In the install this moves ten tracks and one patch:
- the nine gnome tracks (5249 to 5265) now run from
  `ExpansionShared\Sound\gnome`, not `ExpansionPack\xpack`;
- track 6403 and patch 6400, which two downloads both define, now come
  from the download registered last.

## Not ported, and unverified

- **Not ported:** an object replaced by one of the other kind.
  - Track 68262's `[Track]` line comes before a `[Patch]` line with the
    same ID in its file, so in the original the event that plays it
    finds a patch object.
  - Patches 6401 and 6402 are replaced by a later download's tracks.

  What a start does with a patch object, or a hit list with a track
  object, is not traced. Here a track keeps the module of its last
  `[Track]` line, and a patch that of its last `[Patch]` line.
- **Unverified:** the order of the downloads among themselves. That
  decides track 6403 and patch 6400. Here it is by name.
