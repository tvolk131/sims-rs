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
    "Patch" and "Track" at 0x57affb and 0x57b02c);
  - so does every station patch, when the stations are built after the
    sound tables (0x574f0e);
  - an ID registered again replaces the object in its node, which keeps
    its place.
- The map starts with 193 buckets. Before each insert it grows for one
  more entry (0x5813bd calls 0x5823a5). When that is more than the
  bucket count, the count becomes the first prime of the table at
  0x666474 not below it.
- A new node goes at the head of its bucket, the ID modulo the bucket
  count. Growing takes each old bucket in turn and its nodes from the
  head, and puts each at the head of its new bucket.
- The unpause walks the map bucket by bucket, each from its head
  (0x57cd56, 0x5807c2, 0x576be5), and lets each object go.

## What changes here

- `sims_audio::registry::Registry` replays those inserts. `Hot` keeps
  the IDs of its `[Patch]` and `[Track]` lines in file order. The player
  registers each module's in turn, then the station patches as
  `Player::add_stations` adds them.
- The install registers 21,809 IDs from the tables and 366 station
  patches: 24,593 buckets. Tracks run up to 114,390, so the order is not
  their numbers.
- `Player::unpause` lets go of the running tracks in the registry's
  order. A track no module lists goes last, by number.

## Unverified

- **Unverified:** the order the original loads its modules in. Here it
  is `Module::load_all`'s, by name. It only matters for tracks that
  share a bucket, and for the three IDs both a patch and a track use.
- **Unverified:** that the original's station patch numbers stay clear
  of the tables' IDs. Here they count up from `stations::FIRST_ID`. The
  count alone sets the bucket count, and 22,175 is below the next
  growth.
