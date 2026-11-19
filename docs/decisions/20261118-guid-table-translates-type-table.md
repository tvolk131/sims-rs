# A house's GUID table translates its type table

*Decided 2026-11-18.*

The family import saves a lot under the GUIDs its family had when it was
exported, plus a `Gtab` chunk pairing each exported GUID with the GUID
the import gave the new type (0x4297f6). The house read applies the table.

**Reading.** 0x43dd76 reads `Gtab` chunk 1 (0x43ddc2, 0x41578e). It is a
saved-resource header, a 32-bit count, then the pairs as old GUID, new
GUID (0x4158a4). The read skips the header's version and its "batG" tag
without checking them. The payload's first byte gives the byte order
(0x464ec1): when it is nonzero, every number is stored big-endian
(0x464970). A file without chunk 1 gives an empty table.

**Where it is used.** Only one place translates through the table: each
GUID in the type table goes through 0x415751 before its definition is
looked up (0x43a6fb, then 0x43ac10). The family import also reads the
table, through 0x41578e at 0x429652. It appends its own pairs (0x429754)
and writes the combined table back (0x4297f7). So importing a family file
that already carries a table keeps the old pairs ahead of the new ones.
The import's port (#257) has to do the same.

**Translating (0x415751).** The first pair not yet used whose old GUID is
the current GUID replaces it. The pair is marked used, and the search
starts again from the first pair, until no unused pair matches. So
translations chain, and a cycle ends once each of its pairs is used.

**The port.** `sims_formats::lot::GuidTable` reads the chunk.
`sims_content::House::load` translates the type table's GUIDs as soon as it
reads them, before the saved objects are matched to definitions, and keeps
the table in `House::guid_table`. The boot no longer refuses such a house.
Nothing else in the house read uses the table, as in the original.

The first neighborhood's Old Town homes (56, 63, 69, 76 and 77) now load
with every family member resolved. House 69 also boots in strict mode and
steps (`tests/boot.rs`). No base-game house has a `Gtab`, so the
strict Goth run is unchanged.

**Unverified:** whether a malformed table fails the original's house read.
Here it is an error naming the chunk.
