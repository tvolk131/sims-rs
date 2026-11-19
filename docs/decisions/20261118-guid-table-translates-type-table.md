# A house's GUID table translates its type table

*Decided 2026-11-18.*

The family import saves a lot under the GUIDs its family had when it was
exported, plus a `Gtab` chunk pairing each exported GUID with the GUID
the import gave the new type (0x4297f6). The house read applies the table.

**Reading.** 0x43dd76 reads `Gtab` chunk 1 (0x43ddc2, 0x41578e). It is a
saved-resource header ("batG"), a 32-bit count, then the pairs as old GUID,
new GUID (0x4158a4). A file without chunk 1 gives an empty table. The table
is used in one place only: each GUID in the type table goes through
0x415751 before its definition is looked up (0x43a6fb, then 0x43ac10).

**Translating (0x415751).** The first pair not yet used whose old GUID is
the current GUID replaces it. The pair is marked used, and the search
starts again from the first pair, until no unused pair matches. So
translations chain, and a cycle ends once each of its pairs is used.

**The port.** `sims_formats::lot::GuidTable` reads the chunk.
`sims_content::House::load` translates the type table's GUIDs as soon as it
reads them, before the saved objects are matched to definitions, and keeps
the table in `House::guid_table`. The boot no longer refuses such a house.
Nothing else reads the table, as in the original.

The first neighborhood's Old Town homes (56, 63, 69, 76 and 77) now load
with every family member resolved. No base-game house has a `Gtab`, so the
strict Goth run is unchanged.

**Unverified:** whether a malformed table fails the original's house read.
Here it is an error naming the chunk.
