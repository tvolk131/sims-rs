# Opening a neighborhood imports its Patch families after house 0's reload

*Decided 2026-11-19.*

`sims_sim::patch_import::open` does what opening a template
neighborhood does to its records and families before the view first
shows. That is house 0's reload, then the import of the `Patch` folder's
`*.FAM` files, one after another. It works on the neighborhood as read
from its file (`sims_content::Neighborhood`, the formats types the
family import already takes) and leaves it as the save after the import
would write it.

## The order, from the original's code

- The neighborhood view's update (0x557fd3) first calls 0x55c958 while
  its byte +0x1b0 is set. That loads house 0 (0x55cb41 → 0x428843 →
  0x428041 with house 0). The house load starts with the reload
  (0x424806, only called from 0x428064).
- A later update, with +0x1d5 set, calls 0x42c1d5. That asks whether an
  import is pending: the expansions mask (`[0x6e48bc]+0x66`) has
  Unleashed's bit 0x20, and the neighborhood's option byte +0x1aa is
  set. If so, it calls 0x42c0ff.
- The option byte is NGBH option 10. The reader stores it from version
  69 on (0x42559e), and sets it to 1 below (0x4255ae). UserData's NGBH is
  version 73 with the byte clear. UserData2 and UserData3 are version
  63, so their import is pending.

So the import sees the records the reload left.
- A template neighborhood saves no record of TemplateDog or TemplateCat:
  UserData2's and UserData3's NBRS have TemplatePerson only.
- The clone looks the template's record up (0x42571f, from 0x4260e1),
  and that lookup makes none. It fails at 0x426335 without one.
- The reload makes those records. In UserData2 it makes 270: the file's
  26 empty slots from the last, then 244 new slots after its 49. Each is
  a registered definition of a recorded kind that has no record.

Sol's install test of the import (#257) faked the three template
records. This puts the reload's step in their place.

## What `open` does

1. **House 0's reload, as far as the import reads it.**
   - A record whose type no definition has fails, as in
     `World::read_neighbors`.
   - Each family's members with no record are dropped.
   - Each registered definition of a kind in `RECORDED_KINDS` that no
     record is linked to gets a record. The rule is `Neighbors::make_record`'s:
     the last empty slot, else a new one, with ID its place plus one,
     revision 0, no person data and no rows.
   - The record's name and the fields the file saves around it are not
     modeled.
   - Person data word 61 is rebuilt (0x425244): each family writes its
     ID into the first record of each member's GUID. The families' order
     is not kept, so a GUID in two families fails loudly.
   - A neighborhood without family 0 fails loudly: the reload adds one,
     which this path does not port.
   - A family ID that is negative as a signed word fails loudly. The
     reader skips 0xffff and keeps the rest negative (0x424bb6), and the
     pre-step's signed max passes over them.
   - A family other than family 0 left with no members fails loudly: the
     house load deletes it (0x428071 to 0x4280b1, 0x425efb).
   - The file counter starts at 0 (0x424e8e).
2. **Nothing more without Unleashed's bit or the pending option.**
   - The option is NGBH #1's (0x424a1a).
   - Without NGBH #1, the reload reads no records (0x424a90), and
     `Neighborhood::open` keeps none.
   - Without NGBH #1, whether an import is pending is the constructor's
     (0x4244bc), and that fails loudly.
3. **The pre-step (0x42c122 to 0x42c1ad).** The family whose ID (+0x10c,
   its FAMI chunk ID) is 4000 takes the largest ID plus one, so that
   `Strays_4000` keeps 4000. IDs are unique, so the reload's order does
   not matter here.
4. **The scan (0x42bca4).** Each file is imported with
   `family_import::import`, in the order given, and its records, family,
   displaced family and data words are applied before the next, and word
   61 is rebuilt again (0x429873 → 0x425244). The character file numbers
   and GUIDs carry over between files.
   - The family a lot's import displaces keeps its worth. 0x42920f clears
     its house, so 0x4272c5 (from 0x410271) gives the budget plus the
     architecture value. That becomes the budget, and then the
     architecture value is zeroed (0x429226). `family_import` only zeroed
     it before.
5. **The option is cleared (0x42c1ba).** The save (0x4252cc), the dialog
   and the files written are not modeled. The caller takes the made
   types, records, families and lots from the result.
   - The families' friend counts stay as the file saved them. The save
     counts again those that are out of date (0x42542e), and a lot load
     marks every family out of date (0x41003c), so they are counted again
     there too.

## Checked with the original's GUIDs

The original imported the same six files into the first neighborhood.
- Its character numbers give the order: Burb User00012–19, Charming
  24–25, Goth Sr 26–29, Hick 30–34, Kat 35–39, Strays 40–47. That is name
  order, the order NTFS lists the folder in.
- Each housed family's `Gtab` #1 pairs its people's exported GUIDs with
  the new ones. The strays' new GUIDs are those of User00040 to 47.

The install test opens UserData2 and feeds those GUIDs as the draws. It
compares the neighborhood as the open left it, after every file.
- Every lot's `Gtab` #1 equals the first neighborhood's house's, byte for
  byte.
- Every family equals the first neighborhood's under the same ID:
  version, house, budget, architecture value, friends, flags, members
  and tail. The one exception is the number, 8 lower in every family:
  the first neighborhood held more families before its import.
- Every record equals the first neighborhood's of the same GUID:
  version, mode, the saved fields and person data. Its relationship rows
  match too, with their values. The IDs and row targets are 18 lower in
  every record: the first neighborhood held 18 more slots before its
  import.
- Each record is named for its own file.
- Each of the 28 made types is a first-neighborhood character copied
  from the same template (OBJD byte 0x8c).

**Unverified:**
- the records the reload makes in a template neighborhood, against a
  capture;
- the order the original's VM lists the `Patch` folder in. The first
  neighborhood shows name order on Maxis's machine. Track 2's UserData2
  capture will show the VM's.
