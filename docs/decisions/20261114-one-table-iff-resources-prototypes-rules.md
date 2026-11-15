# One table of IFF resources, with the prototype's rules (unverified)

*Decided 2026-11-14.*

`Iff::get`, `Iff::all` and `Iff::resource_order` now read one table built
when the file is parsed, as the original's IFFResFile3 keeps it. Before,
`get` and `all` scanned the raw chunks: they returned deleted chunks, and
`get` took the first of two chunks with one key even when the resource
map named only the second. `Iff::chunks` still lists every chunk in file
order, for readers that want them all.

The rules follow the frozen prototype's port of the open
(`legacy/crates/sims-sim/src/object_catalog/iff.rs`):

- A resource's key is its type, its ID and the high byte of its flags.
  `get` looks up the key whose high byte is 0, as the original's lookup
  by ID does.
- Without a resource map: live chunks in file order. A deleted chunk
  (flag 4) is skipped. A later chunk with a taken key replaces the
  earlier one in its place in the order.
- With a version-0 map: the map's entries in its order. Of two entries
  with one key, the first is kept and the second dropped unread. A map
  listing a resource map with ID 0 is ignored and the chunks are read as
  if there were none. An entry's chunk must be at its offset with its
  type, ID and flags high byte; the low flag bits are the map's to state,
  as in the prototype. (The prototype checks only the type, when it
  loads the payload; requiring the ID keeps a resource's chunk agreeing
  with the key it is found under. The game's own maps always agree.)
- A resource map is never a resource itself.
- When the map cannot be used, `resource_order` fails as before, and
  `get` and `all` use the table built as if there were no map.

**Unverified** against the original. What the game's own files hold
bounds the effect: all 4,155 deleted chunks in the install are `XXXX`
free space, none sharing a key with a live chunk; the only live chunks
sharing a type and ID are nine `BMP_` chunks that the flags' high byte
tells apart; and no map lists a map, a duplicate key, or leaves a live
chunk out. The install census and the gated digests are unchanged.
`sims-vm`'s `Program` still reads `Iff::chunks` directly (a later chunk
of one ID wins there, deleted ones included); on the game's own files
that agrees with the table.
