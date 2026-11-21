# The unpause follows the sound registry's buckets

*Decided 2026-11-20.*

An unpause lets go of every running track, and the tracks then run in
that order. The player had them in order of their numbers. It now
follows the original's registry of patch and track objects.

## The original

- The sound manager keeps every patch and track object in one SGI
  `hash_map` at manager +0x80, keyed by ID (0x57cdd5):
  - every `[Patch]` and `[Track]` line of a HOT file registers one, as
    the HOT reader reads it (0x57b33b and 0x57b676, the handlers named
    "Patch" and "Track" at 0x57affb and 0x57b02c). Both skip ID 0
    (0x57b377, 0x57b6b2);
  - so does every station patch (0x574f0e);
  - an ID registered again replaces the object in its node, which keeps
    its place. That path does not insert, so the map does not grow for
    it (0x57cdf8 to 0x57ce32).
- The order of registration at startup (0x41437f):
  1. the base game's tables, in `SoundData` (0x5715fa);
  2. the station patches (from 0x57183b);
  3. the expansions' tables, from `ExpansionPack\`, then the `sound\`
     directories of `ExpansionPack2` to `ExpansionPack4`, `Deluxe`,
     `ExpansionPack5` to `ExpansionPack7` and `ExpansionShared`
     (0x414392 to 0x414458);
  4. the downloaded objects' tables (0x414477).
- The map starts with 193 buckets. Before each new ID it grows for one
  more entry (0x5813bd calls 0x5823a5). When that is more than the
  bucket count, the count becomes the first prime of the table at
  0x666474 not below it.
- A new node goes at the head of its bucket, the ID modulo the bucket
  count. Growing takes each old bucket in turn and its nodes from the
  head, and puts each at the head of its new bucket, so a chain is not
  simply newest first.
- The unpause walks the map bucket by bucket, each from its head
  (0x57cd56, 0x5807c2, 0x576be5), and lets each object go.
- The base load (0x5715fa) takes the files directly in `SoundData`,
  not its subdirectories (0x5d8729). The expansions' calls each test
  that expansion's bit; the full install takes all nine.
- The downloads' directories are the parent directories of the
  download objects the catalog registered, each once, in the order the
  catalog met them (0x43bdc5).
- The map is cleared only when the sound is torn down (0x57ca76) and
  destroyed (0x57c712), not on a lot load. Kind 104 tears the sound
  down and builds it again from the base tables and the stations
  (0x5742a3, 0x5742b6), so an unpause after it walks that new map. This
  player does not port kind 104.

## What changes here

- `sims_audio::registry::Registry` replays those inserts. `Hot` keeps
  the IDs of its `[Patch]` and `[Track]` lines in file order, but for
  ID 0.
- The player registers the modules directly in `SoundData`, then the
  station patches when `Player::add_stations` adds them, then the
  expansions' modules by directory (`Registry::rank`), then the others.
  Its own module order, which picks events and patches, is unchanged.
- The install registers 21,809 IDs from the tables and 366 station
  patches: 24,593 buckets. Tracks run up to 114,390, so the order is not
  their numbers.
- `Player::unpause` lets go of the running tracks in the registry's
  order. A track no module lists goes last, by number.

## Unverified, and not ported

- **Not ported:** the player registers every module outside the base
  and expansion directories last, not only those in the directories of
  download objects the catalog registered. With the stock install they
  are the same. An install with sound tables in a directory no
  registered object uses would register more IDs here, which can move
  the map's growth.

- **Unverified:** the order the original takes the files of one
  directory in, and the downloads' directories in. Here they go by name.
  It only matters for tracks that share a bucket, and for the three IDs
  both a patch and a track use.
- **Unverified:** the original numbers station patches from 256-ID
  batches starting at 0x400000ff (0x57d386); here they count up from
  `stations::FIRST_ID`, shared with their sub lists. Only how many there
  are, and when, moves the tracks: both number ranges stay clear of the
  tables' IDs.
