# The expansion-pack lot is House 69 after the Patch import

*Decided 2026-11-18.*

Roadmap step 6 asks for one expansion-pack lot that agrees with recordings
of the original. The lot is House 69 in the second neighborhood
(`UserData2`): the Charming family from Unleashed, one adult and one dog.

## Why this lot

- **Community lots** (Downtown, Vacation, Studio Town, Magic Town and Old
  Town's shops) have no family. The original reaches them only by taxi with
  a visiting family. That needs travel, the visitors and the townies of
  `TemplateNPCs`, which is more than a first expansion lot should take on.
- **The Old Town homes with families** are houses 56, 63, 69, 76 and 77.
  None of them loads today, and each fails on a missing object definition
  for one of its people or pets:
  - In the first neighborhood the families are already in the houses, but
    none of their character files are in `UserData/Characters`.
  - In neighborhoods 2 to 8 the houses name placeholder pet types (House
    69's is 0xcb6df838) that no file in the install defines. Each of
    these neighborhoods has a `Patch/` folder holding the six Unleashed
    family files.
- **House 69's family is the smallest of the five.** Its object types come
  from the base game, `ExpansionPack5` (Unleashed in this install),
  `ExpansionShared` and `Downloads`. Its objects don't use the version 3
  TTABs of `ExpansionPack.far` or the truncated TTABs of
  `ExpansionPack6.far`.
- **No new primitive is needed.** The expansion folders' trees use opcodes
  0 to 51, all ported except UI Effect (34, tutorial and help only) and 38
  to 40 (unused). Unported modes will show only in a run.

House 76 (the Hick family: four dogs and an adult) is the fallback.

## The Patch import

When a neighborhood opens, the original imports its `Patch/*.FAM` files
(0x42c1b5 calls 0x42bca4; the folder is asked for at 0x42bd13). The
per-file loader starts at 0x428f05. For each family member it reads the
first body string:

- "dog" or "cat" becomes a dog or a cat if the game's pets option is on
  (`[[0x6e48bc]+0x66] & 0x20`); without it the member is dropped
  (0x42601e);
- anything else is a person.

It clones TemplatePerson, TemplateDog or TemplateCat through 0x4260e1, the
routine Make New Character uses, then writes the member's body strings,
name and description. It pairs each old GUID with its new one
(0x42972e to 0x429799), copies relationships under the new neighbor IDs
(0x429bf1), writes the lot through a `.tmp` file, and saves the
neighborhood (0x4252cc).

The clone gives each new type a GUID drawn from the clock and the C
runtime's generator (0x4725d5). The original's import result therefore
can't be reproduced byte for byte. Our import's check is that booting from
the original's imported files equals booting from neighborhood 2 plus our
import, fed the GUIDs the original drew. The import order, the draws and
the house file it writes are **unverified** until track 2's before-and-after
snapshot of neighborhood 2 exists.

## Order of work

- **6a.** Boot House 69 in strict mode from the original's imported files,
  then compare it with a 1000-tick recording of the original.
- **6b.** Port the import: the family-file reader (#252), the import's data
  step, then the import at neighborhood open in the boot.

What would reopen this: the original failing to import or to load House
69, or the snapshot showing that the import keeps the exported GUIDs.
