# A track runs from the module that registered it last

*Decided 2026-11-20.*

When two modules define the same track, patch, hit list or track data
ID, the player used the requesting module's definition, or else the
first module's by name. The original keeps one table of each for all
modules, so every request gets the definition that registered last. This
port now does the same.

## The original

- **Tracks and patches.** Each `[Track]` and `[Patch]` line registers an
  object in the sound manager's registry (0x57cdd5; see
  `20261120-unpause-follows-the-sound-registry.md` for the order). An ID
  registered again destroys the old object and keeps the new one in its
  node (0x57ce04 to 0x57ce32), whatever its kind.
  - A start looks the track up by number (0x573312) and runs from that
    object's own descriptor (0x57db7a to 0x57db91), whichever module's
    event asked. Its argument kind comes from the same object (0x573753,
    0x57db01).
  - A kill clears the object's running flag (0x57dd4c) but keeps the
    object, so a replay after a kill runs the same descriptor.
  - A start that finds a patch object starts nothing: the patch's slot
    returns at once (0x57bf80).
  - A note whose patch is a track object fails (0x57f6ac to 0x57f703).
- **Track data.** Each `[TrackData]` line stores its address in the
  program being loaded into one table for every module (manager +0x168,
  0x57ce70 to 0x57ce98). A later line with the same ID points into the
  later module's program.
  - A track's entry point looks its ID up there (0x57dd8d, 0x57dbe2).
  - So do `jump`, `call` and the `if` branches (0x57e3b5, 0x57e3d4). A
    thread can therefore go on in another module's program.
- **Hit lists.** The HOT reader makes a new list for each `[HitList]`
  line (0x57ba20), and the table keeps the newest for its ID (0x57d261).
  `smart_setlist` (0x57eec4) and kind 53 (0x573f1b) read that table.
  - The stations add to the lists that exist when they are built
    (0x574c9e to 0x574cb7, 0x574e74). That is after the base game's
    tables and before the expansions'.

## What changes here

- `Hot::registered` records whether each line is a patch or a track.
- The player takes each track, patch, track data ID and hit list from
  the module that registered it last. The order is `SoundData`, then the
  expansion directories in the original's order, then the downloads.
  - An ID last registered as the other kind is that kind only. Starting
    it as a track does nothing and reports nothing. Using it as a patch
    is a fault, and plays nothing.
- Every start, restart and argument kind (`kArgsVolPan`,
  `kArgsIdVolPan`) uses the track's module. There is no longer a
  fallback to the requesting module or the running thread's module.
- A thread runs in the program its entry point, or its last `jump` or
  `call`, led to. A `call` returns into the caller's program.
- The stations' hit lists are added between the base game's lists and
  the expansions'.
- Events still come from the first module that defines them. This
  change does not touch event lookup.

## In the install

- The nine gnome tracks (5249 to 5265) run from
  `ExpansionShared\Sound\gnome`, not `ExpansionPack\xpack`.
- Track 2953, which two modules define, runs from the one registered
  last.
- Track 6403 and patch 6400, which two downloads both define, come from
  the download registered last. Eight more patch IDs change module but
  keep the same sample path.
- `ExpansionPack\xpack` and `ExpansionShared\Sound\gnome` define the
  same 99 track data IDs. So 164 `xpack` tracks enter the `gnome`
  module's program, and 63 jumps, calls and branches land in another
  module's program. The two programs are byte for byte the same, so
  nothing sounds different.
- The 125 hit list IDs that several modules define have the same entries
  everywhere.
- `ltub_running` starts nothing: a later `[Patch]` line in its file
  registers its track 68262 again as a patch. Patches 6401 and 6402 are
  replaced by a later download's tracks, so notes of those patches fail.

## Unverified

- The order of the downloads among themselves. It decides track 6403
  and patch 6400. Here it is by name.
- The cross-kind cases are not traced for the radio's note cancelling,
  kind 53 or the piano. There, an ID whose object is a patch counts as no
  track.
