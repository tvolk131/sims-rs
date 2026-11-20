# Family import can write character IFFs

*Decided 2026-11-19.*

`family_import::clone_character` materializes a made type from the selected
template and its source FAM. `iff::write` serializes the live chunks to a fresh
IFF 2.5 with a zero resource-map offset. It writes a 76-byte chunk header with
big-endian size, ID and flags, and a 64-byte Windows-1252 label field. The file
can be installed as `Characters/User%05d.iff` and read through `Index` and
`Catalog`. The writer chooses its layout independently of the original's
in-place resource allocator.

`0x4260e1` calls the template resource copy at `0x42629e` (`0x4722b8`).
For each OBJD, `0x4725e4` puts the template GUID at byte 0x8c and
`0x4725f3` puts the drawn GUID at byte 0x1c. The pure importer supports the
three single-definition person and pet templates. It reports multiple OBJD
copies at `0x47249d` and legacy `cst` conversion at `0x4723e1`.

The original replaces STR# 200 with the selected uChr entries at
`0x4293d9–0x429444`. It writes language 1 and empty comments and label.
It reads the name and description from the member's CTSS at ID + 2000
(`0x42945d–0x4294f0`), with the same-ID fallback at `0x429516–0x429573`.
`0x40636e` writes the two strings to the CTSS ID in OBJD byte 0x52, which
is 2000 in these templates. `0x470696` updates the active language, keeping
existing comments and translations in the destination. `0x4707be–0x4708de`
fills missing rows from English in languages with their own entries. The
write clears the label. `0x4063df–0x4063f8` removes a legacy CATS at the
same ID. `0x429596–0x4295ee` and `0x4572b6` label the OBJD with the lowercase
file stem and the name, capped at 63 bytes. The clone uses the original
string bytes from the FAM and preserves the remaining template resources.

The native STR# and CTSS writers reserve an extra byte per row at `0x46fc9f`
and `0x47090e`. Their entry loops leave those bytes unwritten at the end of
the payload. The fresh writer fills that space with zeroes. The install test
compares the CTSS size and every byte through its last declared entry.

**Unverified and outside the clone step.** The port ends at the label write.
At `0x4295f7`, the original refreshes the live appearance with `0x457f60`.
`0x587b4b` appends empty rows through row 35 and saves the table. The refresh
can then bind the wardrobe at `0x458059` (`0x409315`) and save the derived
outfits at `0x45807d` (`0x409492`, `0x40a588`). Matching that write needs the
original's ordered wardrobe and appearance state. In the installed oracle
the differences are confined to one-based STR# 200 rows 17, 28–30 and the
added row 35. The test compares the clone's body payload against uChr and
the rest of its rows against the saved character file. Locale behaviors
beyond the installed English import are unverified.

The portrait follow-up calls `0x4e8c5d` at `0x429b1a` and `0x4e2c4e`.
It renders BMP_ 2002–2007 and stores the results through `0x4e9000` at
`0x4e8d2e`, `0x4e8dd7`, `0x4e8e5f`, `0x4e8ed5`, `0x4e8f3a` and
`0x4e8fa6`. The clone omits those slots, including template placeholder
images. The member's exported BMP_ is a separate image at the member ID.

**Oracle coverage.** The six installed UserData2 Patch files contain 28
EXPi members. House 63's Gtab maps Burb's four draws to User00012, User00016,
User00018 and User00019. The other four house tables and the eight ordered
strays map to User00024 through User00047. User00013–00015, User00017 and
User00020–00023 have no mapping from these FAMs and have extra behavior
resources. Their source is unverified. The install test checks every
mapped clone and loads all 28 fresh files through Index and Catalog.
