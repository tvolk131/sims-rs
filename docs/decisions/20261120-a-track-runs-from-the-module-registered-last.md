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
  - Elsewhere a patch object still counts as an object of that number.
    The radio's note cancelling goes on past it (0x573a29 to 0x573a5c).
    The piano still asks for the skill and starts the skill track
    (0x573e66 to 0x573ef2); the instance it records is the patch's
    0x80000000 (0x573e9c calls 0x57bf4e, which reads +0xb0, set at
    0x57bf0e to 0x57bf19). Kind 53 still picks from its hit list, then
    calls the patch's slot (0x573f1b to 0x573f2b, 0x57fcd2).
  - The station patches register through the same routine (0x5750dc),
    after the base game's tables and before the expansions'. Their
    allocator does not look at the registry (0x57d386). So a station
    patch replaces a base game object of its number, and an expansion's
    or a download's object replaces a station patch. Replacing an
    object destroys it, with what it was doing.
- **Track data.** Each `[TrackData]` line stores its address in the
  program being loaded into one table for every module (manager +0x168,
  0x57ce70 to 0x57ce98). A later line with the same ID points into the
  later module's program.
  - A track's entry point looks its ID up there (0x57dd8d, 0x57dbe2).
  - So do `jump`, `call` and the `if` branches (0x57e3b5, 0x57e3d4). A
    thread can therefore go on in another module's program.
- **Hit lists.** The HOT reader makes a new list for each
  `[GlobalHitList]` line (0x57b08e, 0x57ba20), and the table keeps the newest for its ID (0x57d261).
  `smart_setlist` (0x57eec4) and kind 53 (0x573f1b) read that table.
  - The stations add to the lists that exist when they are built
    (0x574c9e to 0x574cb7, 0x574e74). That is after the base game's
    tables and before the expansions'.

## What changes here

- `Hot::registered` records whether each line is a patch or a track.
- The player takes each track, patch, track data ID and hit list from
  the module that registered it last. The order is `SoundData`, then the
  expansion directories in the original's order, then the downloads.
  The station patches go between the base game's and the expansions'.
  - An ID last registered as the other kind is that kind only. Starting
    it as a track does nothing and reports nothing. Using it as a patch
    is a fault, and plays nothing. The radio, the piano and kind 53 treat
    a patch object as the original does.
- Every start, restart and argument kind (`kArgsVolPan`,
  `kArgsIdVolPan`) uses the track's module. There is no longer a
  fallback to the requesting module or the running thread's module.
- A thread runs in the program its entry point, or its last `jump` or
  `call`, led to. A `call` returns into the caller's program.
- The stations' hit lists are added between the base game's lists and
  the expansions'.
- Stations added after sound has played destroy what they replace. (The
  original builds them before any sound plays.)
  - Each station patch is a new object (0x5750a0 to 0x5750c5), even
    with the same sample as the one it replaces.
  - The old object's threads, its notes and its cached samples go.
  - A note playing another patch, with the destroyed one queued after
    it, plays on without it (0x57e09e to 0x57e0b6, 0x57fd25).
  - The manager's instance records stay (0x57442d to 0x574473).
  - A kind 30 event records its instance even when its number is a
    patch object (0x573d36, 0x573d5a).
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
- Whether a voice still plays a patch that was destroyed while queued
  on it, from the path it copied (0x57ff7e to 0x57ff9a). Here it does
  not.
- No station patch number collides with a table's ID in the install, so
  the station rules above change nothing there.

## Not ported

- Kind 53's patch slot (0x57fcd2) can stop the patch's playback and let
  go of its samples. Here a kind 53 event that names a patch object
  picks from its hit list and does nothing more. No kind 53 event in the
  install names a patch.
- Kind 1 on a number that is a patch object asks whether the object
  plays (0x573719; a patch's slot says no, 0x4eefbd) and then removes
  the number's instance records (0x57372a). Here it returns before that,
  so those records stay.
- Kind 30 on a patch object records the instance, as the original does
  (0x573d36). The original then also works out the placement and sets
  the volume and pan (0x573d48, 0x573d64, 0x573d6e). Here it stops after
  the record.
- In the install, `ltub_running` (kind 1, track 68262) is the only
  track event that names a patch object. No event records instances of
  68262, so the missing cleanup changes nothing there. No kind 30 event
  names a patch object.

## Left as it was

- A `jump` (opcode 13) in the original always reads a four-byte operand
  (0x57e3ae). This player still reads a register operand when the four
  bytes are not a track data ID.
  - All 401 jumps in the install decode the same whether that check
    uses the jump's own module's track data or the shared table, which
    is what this change could affect.
  - Two of them still take the register form. Reading four bytes there
    instead breaks one program's disassembly, so that form stays.
