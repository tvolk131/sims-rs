# The queue's icons show their objects' catalog pictures

*Decided 2026-11-15.*

An object's icon in the action queue now shows its type's catalog
picture, as the original makes it (`sims_ui::thumbnail`). The earlier
queue entry said the original renders the picture from the object's
sprites. That is only its fallback. Read from the code:

- **Which types have one** (0x4cd830): each definition in an object file
  gets a catalog item (0x4cbd44, kept at definition +0x84). The
  exceptions are a multi-tile object's tiles other than its lead, and
  people. A definition also needs graphics (0x457588) or an interaction
  table with entries (0x4575a1, 0x457cb0): the `TTAB` its `OBJD` word at
  byte 0x0e names, from its own file or else its semi-global file. Stock
  `ReserveTile` has neither, so it has no item (a Goth test checks
  this). An object shows its own item, or for a multi-tile object, its
  file's lead tile's (0x45728d, 0x43ae0a: same master ID, sub-index -1).
  An object without one shows the dark grey square, as before.
- **The thumbnail** (0x4cbdc3): the file's `BMP_` with the definition's
  catalog strings ID (`OBJD` byte 0x52). For a type with graphics it is
  kept only if it is 74 by 37: two 37 by 37 cells, the object on the
  catalog's normal and highlighted backgrounds. Of the install's 1,835
  items (one per lead definition), 1,656 ship a thumbnail that is kept,
  and none uses the magenta key color; 69 types that are not people have
  no item. A thumbnail that does not decode is reported, and its item is
  kept without it, as the original keeps the item.
- **The icon** (0x4ccaa0): the first 45 by 45 cell of `kCatalogTemplate`
  (4300, `CPanel/Catalog/ThumbTemplate.bmp`), then the thumbnail's left
  half (when wider than 45) centred over it, 4 pixels in for a 74 by 37
  one. The icon frame goes over that.
- **Verified (a Goth test,
  `the_queues_icons_show_their_objects_catalog_pictures`):** the queued
  fridge's icon and the running snack's show their own thumbnails' left
  cells, pixel for pixel, inside the frame.
- **Not ported:** the render (0x4e57a6) of a type whose thumbnail is
  missing or the wrong size: 179 of the 1,835, mostly social
  interactions, plus a few objects (cookbook, wall lamps, a display
  case). Such an icon shows the empty template, which is what 0x4ccaa0
  leaves when it has no thumbnail.
- **Unverified:** that the table's word +4, which 0x4575a1 tests, is
  its entries, so that an empty table counts as none. Global 0x6f2a3c,
  which drops every kept thumbnail of a type with graphics, is 0 in our
  capture. No capture shows a queue icon.
