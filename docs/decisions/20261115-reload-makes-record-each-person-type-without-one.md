# The reload makes a record for each person type without one; UserData2 to 8 load

*Decided 2026-11-15.*

The houses of UserData2 to UserData8 failed before any tick: "the person
type 0xA5C5AEE7 has no neighborhood record". It is not a loading bug. We
read the right files (each `UserDataN/Neighborhood.iff`, with
`UserDataN/Characters` registered), but those are the 2000 templates,
never opened by the Complete Collection: 21 to 23 records, against 231
person types of the expansion packs and downloads (0xA5C5AEE7 is
`Downloads/NPC_JonBonJovi`) that have none. The install's `UserData` file
was saved by the original after it had made them, and has 463.

What the reload (0x424806) does to the records is now ported, from the
original's code:

- Reading a record links it to the definition its GUID resolves to
  (record +8, 0x423ef2, the registry's map 0x43ac10). The reload deletes
  each record linked to none (0x424ad8, 0x4259a2: a signal 0xfb, the
  record's inventory emptied of kinds 0 to 9, its slot cleared, the first
  row aimed at it removed from every record). No shipped neighborhood has
  one, so that **fails loudly** instead of being ported. 0xfb's
  subscribers were not checked.
- After the families are read and sorted, it walks the registry's
  vector (registry +0x20, 0x43ac3a), which keeps every definition in
  registration order, duplicates included. Earlier in the reload
  (0x43c228, at 0x424a8b) the neighborhood's characters are registered
  again, which appends them to the vector after every other definition.
  Our catalog has them before `Downloads`, so `registry` moves the
  `Characters` mount's definitions to the end (Sol's review found this:
  in a neighborhood missing records for Jon Bon Jovi and User00122, slots
  18 and 414 would have gone the wrong way round). For each of OBJD type 2,
  0x200, 0x400, 0x800 or 0x1000 (OBJD +0x12) that no record is linked
  to, it makes one (0x4257a5 with slot -1, 0x4237a1). The new record
  goes in the **last** empty slot, or in a new one at the end. Its ID is
  the slot's place plus one. It has the definition's GUID (0x4572af),
  revision 0 and every data word 0, with no rows and no inventory. A
  definition that a later one of its GUID replaced gets a record too,
  since records link to the replacement. The types beyond 2 matter: the
  install has 39 such definitions (36 of type 0x400), which the old
  check missed.
- Families drop members before the records are made, so a member whose
  record is made then stays dropped. Word 61 is rebuilt after.
- The new record's friend count and its out-of-date flag (+0x20, +0x24)
  are not set by the constructor. The original can read them before a
  relationship write sets the flag: scope 24 field 5 (0x425c19,
  0x427148) returns the count as it is when the flag byte is 0. Here
  they are 0 and set, as for a record read from the file. That field
  fails loudly, so nothing reads them.
- The original reads the records only when the file has an NGBH chunk
  (0x424a90); without one it makes them all. Ours reads them either way,
  and every shipped neighborhood has one. The templates' NGBH is an old
  version, which the reader (0x4254f6) takes; below version 0x45 it
  sets the pending-patch flag (+0x1aa).
- Its name (+0x28) is presentation. So is the list of deleted people's
  names (+0x168).

`read_neighbors` now takes the registry
(`sims_sim::neighbors::registry`, from the index and the catalog)
instead of the person GUIDs.

Right after the reload, the house load (0x428041, 0x428071) deletes each
family other than family 0 with no members (0x425efb). No shipped
neighborhood has one, so `World::drop_empty_families` **fails loudly** for
one.

**Verified:** the Goth house's strict output is byte-identical through
tick 2881, hash included (`eed688a0...`, default frames, and with
`--frames none`). The base neighborhood already has a record for every
registered definition, so nothing is made there.

Checked against a capture of the original's registry in the base
neighborhood (`newbie-startup-20261021/live-catalog.json`): with the
characters moved to the end, its 463 definitions of these five types
come in the same order as ours. The whole registry differs elsewhere
(seven definitions we register that it does not, and an order that
parts at entry 646), which the reload does not read.

**Unverified:** the records made for a template neighborhood against a
capture.

What the template neighborhoods still need before they match the
original: opening a template neighborhood imports the families in its
`Patch` folder (`*.FAM`, 0x42bca4, called from 0x42c0ff behind expansion
bit 0x20 of `[0x6e48bc]+0x66` and the neighborhood's pending flag +0x1aa).
That runs from the neighborhood view with a dialog and writes character
files, before any house load. The expansion families of the base file
(2000, 3000 to 3009, 5000, 5500, 6000) come from somewhere else again.
Neither is ported. A recording of a UserData2 to 8 house should start from
a neighborhood the original has already opened and saved, with those
files copied into the cache.

Where those houses stop now:
- All ten of UserData2's base houses are version 63. So are houses 2, 3,
  5, 6, 7 and 10 of each of UserData3 to 8; the rest are 62. Version 63
  fails at the arrays: floors and walls are Arry 11, 12,
  111 and 112 there, which is not ported.
- The version-62 houses of UserData3 to 8 have no family and stop at the
  household selection.
- The only family in UserData2 to 8 lives in UserData2's house 9, which
  is version 63.
- The expansion lots (versions 69 to 73) stop at the object read. Their
  people's definitions are in no registered file.
