# Save IFF resources through the family import's in-place writer

*Decided 2026-11-19.*

The family importer constructs and opens an IFFResFile2 at
`0x428f2d–0x428f40`. The GUID-table write at `0x4297f7` calls
`0x4157b6`, which selects Gtab #1 and stream version zero.
`0x415835` calls the generic resource save at `0x464dbd`.
`0x464cdf` makes a sizing pass and a writing pass over the stream.
For Gtab those passes reach the count and GUID-pair serializer at
`0x41588c–0x4158a4`.

The IFFResFile2 vtable at `0x66682c` sends add through `0x41d01b`
to `0x419ed8`, write to `0x419fb9`, and detach to `0x419e08`.
The add finds the requested type, ID, and variant, falling back to a
matching variant zero. A replacement marks the old chunk deleted and
keeps its resource-table slot. A new resource appends to its type's
vector. A new type appends to the type vector. The add uses the high
byte of the flags as the variant and sets bit `0x10` for converted
data. Labels are byte strings with up to 63 bytes.

## Allocation and close

- `0x41a3ac` changes the erased chunk's tag to XXXX, sets flag `0x04`
  and adds its full length to the unused counter. It preserves the
  ID, label, and payload. `0x419615` uses that operation to retire the
  previous resource map on the first write.
- `0x41a3ec` starts allocation at byte 64 on a fresh open. Each later
  allocation resumes at the preceding allocation's header. The scan
  recognizes free space by flag `0x04` and coalesces consecutive free
  chunks, updating the first chunk's length. It uses the first exact
  fit or the first block with room for the resource and a 76-byte
  remainder header. A scan that reaches EOF appends the new chunk.
- At `0x41a4da` a split writes XXXX, the remaining length, and flags
  `0x04` to the remainder header. Its ID and 64 label bytes come from
  the stack slot that the scan uses for lookahead. A successful
  lookahead supplies the header bytes. The remainder's payload stays
  in place.
- The importer closes at `0x42994d` before moving the file.
  `0x4196c2` can compact a modified file when the signed unused
  counter reaches `0x2800`. `0x41a63d` moves live chunks in physical
  order, adjusts table offsets, truncates the file, and resets the
  cursor and unused counter.

Close serializes the new map through `0x41ad07`, `0x41ac25` and
`0x418c97`, then places rsmp #0 with flag `0x10) through the same
allocator. The map payload starts with two zero words and `pmsr`.
It then contains the little-endian unused count sampled before the
map's allocation, the type count, and one group per type. Each group
has the little-endian FourCC and entry count. Each entry stores a
little-endian file offset, ID, and flags, followed by a NUL-terminated
label aligned to two bytes relative to the payload. The type and entry
order come from the old map's vectors or from the physical scan at
`0x419465` when opening without a usable map. The new map excludes
itself. Its unused count can include space that its own chunk then
occupies.

The file starts with a 64-byte IFF header. `0x419615` writes the
60-byte signature and credits from the template at `0x6c2094`.
The final four bytes in version 2.5 are a big-endian rsmp offset,
with zero while the map is invalidated. The 2.0 reader treats those
four bytes as part of the signature area. The write emits 2.5.
Individual chunks have big-endian size, ID, and flags after their
FourCC and a 64-byte label field.

## Installed comparisons and unverified paths

FAMI links Burb to House63, Charming to House69, Goth Sr to House77,
Hick to House76, and Kat to House56. Burb uses an earlier free run
and leaves the retired map at EOF. Charming consumes the retired map
and the consecutive free chunk. For Goth Sr, Hick, and Kat, Gtab splits
the retired map at EOF and the new map appends.

All five comparisons reproduce every output byte when replaying the
native allocator residue. At `0x464cd7` the map serializer advances
over a pad byte without writing it. Each shipped map has such bytes,
including nonzero values. In the three EOF splits, `0x41a4da` writes
66 bytes from its uninitialized lookahead slot. Those headers differ
from one another. The FAM and Gtab payload supply neither source of
residue. The default writer reports the corresponding address.
The install test permits replay input only at independently located
pad bytes and these split-header ID and label fields.

The compaction and replacement paths have synthetic coverage. Their
output is unverified against a native save capture. Streams beyond
the uncompressed version-zero resource map report an error at
`0x41891d`. Writable-stream failures and other IFF object classes
are outside this single-edit byte API.
