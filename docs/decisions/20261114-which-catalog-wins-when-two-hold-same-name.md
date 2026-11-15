# Which catalog wins when two hold the same name

*Decided 2026-11-14.*

The original's code settles this for appearances and clips. Before
loading, it builds a list of search folders (0x5835ed, as the old
prototype traced it). It then lists the catalog files in those folders
(0x5c1a7d, as the old prototype ported it in
`legacy/crates/sims-sim/src/pose/clothing/catalog_files.rs`):
- every text catalog (`.cmx`) in every folder before any binary one
  (`.bcf`);
- each kind folder by folder;
- a file name already listed from an earlier folder is skipped.

It loads them in that order (0x59f844 for appearances, 0x5a0212 for
animations). For each record it looks the name up first. A name not yet
known is added. A name already loaded keeps its record, and the new one
is released (0x59f087, which calls 0x5954a5 for appearances and
0x59778a for clips). So **the first catalog loaded wins.**

For appearances the folder order is:
1. the game's own skins (`GameData/Skins`);
2. the shared expansion folders (`ExpansionShared/Skins` and others);
3. each expansion's `Skins`, `Animations` and `Accessories`, from the
   newest expansion back;
4. the first expansion.

An archive's members count as being in the archive's folder plus the
folder in the member's name. For example, `Deluxe.far`'s
`Skins\b200mafit_ctb.cmx.bcf` is in `deluxe/skins`.

`sims-people` now does the same. On first use it puts every catalog of
the install in that order, reads each, and keeps the first catalog of
each appearance name. A catalog it cannot read is skipped, as the
original goes on after a file it cannot read. Reading them all takes
under a second. This replaces the old lookup, which:
- tried the binary file named after the appearance first, then the text
  one;
- then took the first catalog in name order.

In the Complete Collection, one appearance is given differently by two
catalogs: `b200mafit_ctb`. It is in `GameData/Skins` as text and in
Deluxe's archive as binary, with different bindings. The original takes
the `GameData/Skins` one, and the old lookup took Deluxe's. A gated test
checks this one, and unit tests check the order and the first-wins rule
without the install.

What is still approximated or open:

- **Order within one folder: unverified.** It is the original's file
  map's, which is not ported; we use name order. No folder of the
  install holds two catalogs giving the same name.
- **Folders the original does not search.** Catalogs elsewhere (such as
  an expansion's object folder) are loaded after all the others. The
  original may not load them at all.
- **Clips.** The rule is the same, with `GameData/Animation` searched
  first. No clip name is defined by two files in the install, so our
  order (the first file in name order) never picks differently.
- **Same-named files other than catalogs: unverified.** 47 file names
  exist in two sources with different bytes: accessory textures such as
  `bottle.bmp`, some animation files shared by the third expansion and
  `ExpansionShared`, and a few meshes. Which copy the original reads
  depends on how it looks those files up, which is not ported. Its
  texture search lists the expansions before `GameData` (0x413ccc), the
  reverse of our order, and the expansions' copies of the accessory
  textures are larger. So the original may well prefer them. None of
  these files is drawn for the Goth family. We keep our order and mark
  it unverified.
