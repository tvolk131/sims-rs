# Refresh imported outfits from the startup wardrobe

*Decided 2026-11-20.*

The caller loads `ImportWardrobe` once for the install, calls
`family_import::clone_character` for each member, then calls
`family_import::refresh_outfits` on the returned chunks before `iff::write`.
This keeps the template copy usable on its own and shares the two ordered
lists across a family. The refresh re-encodes the clone's STR# 200 only after
selection succeeds. Make New Character still reads `Wardrobe` for counts.

The suit registry hashes signed bytes with 32-bit wrapping at `0x5b8ef9`.
It starts at 193 buckets from the prime table at `0x666474`, grows before
checking for a duplicate at `0x5a330e`, and prepends nodes at `0x5a3292`.
Growth walks the old buckets and chains in order and prepends into the new
buckets. Enumeration at `0x5807c2` and `0x40fdf7` visits ascending buckets,
then each chain. The `0x5835ed` search paths feed every CMX before any BCF
(`0x59f844`), with lowercased names (`0x594d73`, `0x595140`). The NDX pass
(`0x584562`, `0x5a0212`, `0x59fc48`) inserts keys in their stored case.
Names beginning with `suit-` participate in every insertion and resize.
`0x409255` filters them out when it builds the picker list.

The texture paths follow `0x414633–0x41506f`, including the object-provider
directories that `0x43bdc5` collects from registered Downloads objects.
FAR members enter their directory's file table in manifest order
(`0x5b9dbc`). `0x5c1a7d` walks each table in bucket and chain order, and
`0x5c11d6` retains the first equal resource name and ID. `0x40895a` adds
BMP entries before TGA entries.

The import at `0x4295f7` enters `0x457f60`, where `0x587b4b` pads the body
table through row 35. `0x409315` upgrades outfits before the exact normal
body and head lookups at `0x40d0a4` and `0x40d408`. `0x409492` validates
and saves the outfit rows through `0x40a588`. The ranked selectors compare
the matched-prefix fraction to a single-precision stored score
(`0x472a25`). In the suit loop, `0x40b304` skips the formal selector as
soon as the nude selector has a match. High fashion comes from `0x40bfeb`
for adults, `ADDED` at `0x409432` for children, and `NONE` at `0x40a614`
for pets. The installed test compares all 35 STR# 200 rows on all 28
mapped imports and permits only the native trailing allocation residue
described in the [character-clone decision](20261119-import-character-files.md).
The RNG fallback reached via `0x40ed83` fails at `0x40ea58`. Legacy outfit
replacement paths outside the replay also return address-bearing errors.

The replay assumes a cold Windows CD Complete Collection startup with
the edition word set to 255, unsplit FARs, the startup OBJD filter enabled,
and the Windows-1252 file names and NTFS uppercase directory collation used
in the research. It infers the ordered provider vector from the mounted
Downloads resources. The source is the installed data and the clone's
language-1 body rows.

A fresh VM capture should check the Win32 enumeration order feeding the
directory tables, the provider vector returned by `0x43bdc5`, and the
resolved appearance, indexed, and texture paths. It should compare the
suit registry after the eager CMX/BCF loads and after NDX registration,
including bucket counts and chain order. It should also check the lazy
suit and texture lists at first use, the per-member body and head lookup
results and wardrobe writes, and whether `0x40ed83` takes the RNG branch.
The asset replay and saved IFFs cover the current oracle. A live startup
capture and the portrait renderer remain unverified by this change.
