# The expansion-pack lot is House 69 of the first neighborhood

*Decided 2026-11-18.*

Roadmap step 6 asks for one expansion-pack lot that agrees with recordings
of the original. The lot is House 69 in the first neighborhood
(`UserData`): the Charming family from Unleashed, one adult (Claire) and
one dog (Luna).

## Why this lot

- **Community lots** (Downtown, Vacation, Studio Town, Magic Town and Old
  Town's shops) have no family of their own. Most are reached with a
  visiting family: by taxi, or for Magic Town by balloon or portal. The
  shipped readme says Studio Town can also be entered from the
  neighborhood screen without a Sim. Either way they need more of the game
  (travel, visitors, the lot's own NPCs) than a first expansion lot should
  take on.
- **The Old Town homes with families** are houses 56, 63, 69, 76 and 77.
  In the first neighborhood each was saved under its family's exported
  GUIDs and carries the family import's GUID translation table (`Gtab`).
  Through it, every member resolves to a file in `UserData/Characters`:
  House 69 maps Claire 0x64BCCDFB to 0xBC364BF5 and Luna 0x67FDDFD9 to
  0xBE24148E. Once the table is read
  (`20261118-guid-table-translates-type-table.md`), all five load.
- **House 69's family is the smallest of the five.** House 76 (the Hick
  family: two adults and three dogs) is the fallback.

## What a strict boot of it reaches

The lot loads in strict mode: 292 saved objects, family 13 selected, funds
1373. The first tick runs, and at tick 3 object 3 shows a dialog with two
buttons. Which button the original's player presses has to come from the
recording. With every dialog answered by its first button the run reaches
tick 495, where Claire's clip "a2o-petbowl-fill" fires a "dress" event
(0x5912fd), which is not ported. More unported modes are expected: for
example the pet pen's BHAV 4113 (`DisplayFloorPetPen.iff`) uses Make New
Character's dog and cat modes, which the port rejects.

The census of the expansion folders only shows that their trees use opcodes
0 to 51. It does not show which modes a strict run will reach.

## The recording needed

Track 2 records the original loading House 69 of the first neighborhood
(no import needed) and running its first 1000 ticks with full-state
captures, noting each dialog and the button pressed. The before-and-after
snapshot of the second neighborhood's import is for 6b only.

## The Patch import (6b)

When a neighborhood opens with the Unleashed edition bit and its pending
byte (+0x1aa) set, 0x42c1b5 calls the `Patch/*.FAM` scan (0x42bca4) and
then clears the pending byte (0x42c1ba). The per-file loader starts at
0x428f05. For each exported member it:

1. tests body string 13 for the substrings "dog" and "cat" (0x4292de,
   0x429311; Luna's is "dogfemale") to pick TemplateDog or TemplateCat when
   pets are enabled;
2. clones the template through 0x4260e1, the routine Make New Character
   uses, and writes the member's body strings, name and description.

What the branch without pets does (it calls 0x42601e with the local
neighbor pointer) is not settled. After the members it:

1. pairs each exported GUID with its new one (0x42972e to 0x429754);
2. copies the relationships between the exported members under their new
   neighbor IDs (0x42975c to 0x429797, 0x429bf1);
3. writes the pairs as the house's `Gtab` (0x4297f6);
4. moves the outgoing house file aside as a `.tmp` (0x4299db) and moves the
   updated family file into the house's path (0x4299ff);
5. saves the neighborhood (0x4252cc).

The clone draws each new GUID from the timer and the C runtime's generator
(0x4725d5), so two runs of the original give different files. A port can
match a given run only when it is fed that run's draws. That is how 6b will
be checked against the snapshot.

## Order of work

- **6a.** Read the `Gtab` (done with this entry). Then boot House 69 of the
  first neighborhood in strict mode and compare it with track 2's
  1000-tick recording, porting what the run reaches.
- **6b.** Port the import: the family-file reader (#252), the import's data
  step, then the import at neighborhood open.

What would reopen this: the original failing to load House 69 of the first
neighborhood, or the recording showing it imports something on that load.
