# Family files join exported character resources through EXPi and NBRS

*Decided 2026-11-18.*

A `.FAM` is a Maxis IFF containing a household and, when present, its
lot. The reader keeps the IFF for later lot loading and uses the existing
`FAMI`, `NBRS` and string-table readers. `FAMs` holds the localized family
name. `FAMh` is optional. The original reads its compressed record vector
at `0x4109a9`, `0x4113d8` and `0x41180a`. The field meanings are unverified,
so the reader keeps the payload as bytes.

The family-file loader starts at `0x428f05`. It reads `EXPi` ID 0 at
`0x428f86` and uses the family ID to load `FAMI` and `FAMs` through
`0x410305`. The stream reader at `0x4243bc` consumes the family ID and
eight neighbor IDs as little-endian words. Version `0x3f` adds a byte
initialized from the game's language at `0x428a90`.

At export, `0x428abc` walks the `FAMI` GUID vector. The lookup at
`0x42571f` finds the first neighbor record for each GUID, and `0x428ad2`
stores the record's ID in the `EXPi` list. The writer at `0x428b81`
uses the same neighbor ID for `uChr`. At import, `0x428f86` loads the
list into `[ebp-0x9c]`. The loop at `0x429271` looks up `uChr` by these
IDs and stops at zero or after eight entries. At `0x42965d–0x429695` each
ID selects a one-based slot in the exported `NBRS` vector. The source
record supplies its GUID at `0x42973b`. `FamilyFile::member_chunk_id`
joins the serialized resources by this key. The `0x42886e` routine and
its callers at `0x427fd9`, `0x427fed` and `0x428001` instead read and
compare lot coordinates.

The exporter writes the portrait under the neighbor ID at `0x428c90`.
The name and description are in `CTSS`. The importer tries ID + 2000 at
`0x42945d` and falls back to the same ID at `0x429516` via `0x4061e3`.
The reader exposes both payloads when present.

The string writer at `0x46fc7a` allocates a four-byte header and the
text length plus four bytes per entry. It writes a language byte, the
text and two NULs, consuming the text length plus three bytes per entry.
The unused bytes remain at the end of the chunk. The localized reader
(`0x46ffcc–0x47006b`) consumes the declared entries and skips the unused
space by releasing the resource after reading. The original IFF retains
those bytes.
