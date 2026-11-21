# Neighborhood open replays the five houseless pools in memory

*Decided 2026-11-20.*

Step 6b E extends `sims_vm::neighborhood_open::open_neighborhood`. After
house 0's reload and Patch, the neighborhood update fills celebrities
(5000), fans (5500) and Magic Town residents (6000), in that order
(0x5580a2–0x5580e9). A supplied lot or toolbar transition then fills
Townies (2000) and tourists (3000–3009), in order
(for example 0x55855d–0x558588). A view-only open ends after Magic Town.

## Families, records and files

- The option pairs are the native offsets 0x1a0–0x1a9 decoded from
  NGBH #1 by 0x4254f6. The first three wrappers test done (0x42c548,
  0x42d4f1, 0x42d941). The last two test pending (0x42b007, 0x42b8d7).
  Each wrapper clears pending and sets done after filling. The direct
  worker call after each wrapper also runs when conversion is complete.
- The first four workers refill to 12, 18, 30 and 30 people, respectively
  (0x42c1ea, 0x42d193, 0x42d5f2, 0x42a98f). The tourist worker (0x42b0fd)
  creates only missing families, in ascending ID order. Its two
  composition rolls choose one or two adults and one or two children
  (0x42b4fe, 0x42b53c). The replay takes the recorded outcome as an ordered
  group of people and checks that its composition is possible. The
  pending Townie and tourist wrappers move preexisting reserved IDs to
  the largest ID plus one (0x42b09f, 0x42b975). In the other wrappers,
  a collision indexes past the family vector (0x42c5e9, 0x42d592,
  0x42d9e2) and the port reports the address.
- A missing family starts at its reserved ID with house, budget and
  flags zero (0x41003c). The new member GUIDs append in creation order
  (0x410849). Character file numbers share Patch's counter at +0x160
  (0x42ce3b, 0x4264f6, 0x426d6e). Occupied filenames advance the counter
  without drawing a GUID. The counter wraps at 32000.
- A person takes the last empty neighbor slot or appends a slot
  (0x4257a5 with ID -1). The editor saves mode 9 (0x425a69). The saved
  form is NBRS version 10, extra 9, with 256 words and empty
  relationships. Word 61 is zero until the family-word rebuild
  (0x425244). The two UD2 files show a rebuild after Magic Town and
  before Townies. `LotTransition` records that boundary explicitly,
  resetting the counter as reload does at 0x424e8e.

## Recorded inputs and CRT

The caller loads the capture and supplies `PoolInput` explicitly. In
the run-2 test, `SIMS_UD2_AFTER/Characters/User00030.iff`
through `User00152.iff` supply the OBJD template and GUID high word
(0x4725e4, 0x4725ff), and the body strings used to derive age, sex and
tone (0x5888d3). The replay writes these files at the allocated numbers.
They carry names and appearance. FAMI in the captured `Neighborhood.iff`
identifies the people in each group, including the tourists'
composition. NBRS supplies the celebrity editor words 10–12, 15,
17–18, 80–82 and the tourist child editor words 9–12, 15, 17–18,
56–57. The test uses the rest of NBRS, FAMI and NGBH for comparison.
Patch still takes its recorded GUIDs from `User00002.iff` through
`User00029.iff`.

The port draws each GUID's low word from the CRT (0x4725ff) and checks
it against the OBJD. Each temporary person copies a skeleton (0x4596f6,
0x598da3, 0x598a6e, 0x595d65), whose file is taken as already read. The
first is made from the pool's template and copies the template's
skeleton (0x426e8e): a tourist child's first actor is an adult. A
second is made after the age and skeleton are set (0x426e97 via
0x40cc2a) and copies the one the profile's age sets. A Townie makes
only the first (0x426646, 0x4269c5). Each skeleton is looked up as
0x5a0384 does, by its name compared byte for byte (0x5a280f;
registration keeps the case, 0x59f163, 0x59e899, 0x59fd58), in the
avatar files `People::load` searches, read record by record. Those files
are not the original's search: it registers `GameData/Skins` eagerly
and loads through `Animation.ndx`. That predates the pools and is a
follow-up. These fail, and only when a pool person needs that skeleton:
- a name no file defines, or two files define (the miss, 0x5a03c0);
- a file whose skeletons do not decode;
- a template GUID with two definitions, since the registry and the
  catalog order them differently.

The skeletons must already be loaded (`PoolInput::loaded_skeletons`). A
skeleton loaded on first use (0x5a03cd) fails, since its draws are not
ported. Both captures fit `adult` and `child` loaded; that is fitted,
not traced. The copy draws by the skeleton's own bone
scales (1,392 draws for the Complete Collection's adult and child).
Each copy is followed by two default-trait passes (0x4598d4 and
0x459a6d into 0x459a9c). Both are conditional; the replay takes both as
running, which every pool person of both captures fits. Townies construct one temporary person at
0x426646 or 0x4269c5. The other pools construct two
(0x42cf4b/0x42cf54, 0x426e8e/0x426e97). The last pass supplies the 15
saved trait words. The run-2 test compares the parsed records 322–444
(all 256 person words and the record's other fields) and the parsed
fields of the 14 pool families, not the saved bytes (FAMI 0x410696,
NBRS 0x423e08).

**What the replay does not check.** It takes the recorded people as
given and checks only some of what the original could have made:
whether a template is the pool's, the GUID's low word, the age and
order, and the tourists' make-up. It does not check that the
first celebrities are the sex their ordinal fixes (0x42c6d7), nor that
a Townie's template is one the specialty draw could still pick
(0x42ac96 to 0x42acc1). Inputs from the original's files pass either
way; hand-made inputs could describe a pool the original cannot make.

| Capture | First three pools | Transition | Townies | Tourists |
|---|---:|---:|---:|---:|
| `after-with-lot-load` | 176,409 | 64 | 44,096 | 102,893 |
| `after-run2` | 176,444 | 64 | 44,096 | 97,027 |

Both captures match the replay's pool families, records and flags. Run
2 made 33 tourists, and the earlier run made 35. In run 2,
`OPEN_DRAWS = 84_947` advances CRT state 1 to the first pool boundary,
covering startup, neighborhood navigation and Patch. The offset is
fitted from the first pool's GUID and trait sequence and assumes no
reseed from state 1. Replay ends at draw 402,578, state 219,850,067.

`Lot::load_after_pools` uses the post-pool CRT state directly. What the
game already holds is a `Handoff` the caller gives: the skeletons whose
files it has read, and whether the indicators exist. With the
indicators made, the lot's indicator objects are made without charging
their 64 graphics-root draws again (0x445e4a, 0x595d65). The run-2 test
gives the handoff that fits: the adult skeleton read and the indicators
made. House 69 adds 3,272 draws on that path and reaches CRT state
3,739,951,803 and the `IMPORTED_69` pin, a regression pin of our run
(`20261120-ud2-house-69-boots-from-the-originals-import.md`). The
handoff is fitted, not traced. Its skeletons are added to those our
load already counts as read before the first lot (the startup cache);
that the original's cache still held them is assumed, since the
registry can clear what it has read (0x5a38b3). The 64 draws the
transition charges before the Townies are a separate input
(`LotTransition`), whether or not they are the indicators'. Their
number is inferred from the fitted schedule, the gap between Magic
Town's last draw and the Townies' first in both captures. Their source
is untraced.
`FILE_BOOT_DRAWS = 401_120` remains the fitted offset for the separate
standalone test from the original's files. Its boot builds the
indicators and reads the adult skeleton file as part of its setup.

## Evidence still needed

The run-2 capture notes and House 69 manifest record the occupied-lot
click that started the last two pools and then loaded the lot. This
points to the normal Unleashed lot callback at 0x5584e8, selected at
0x55ba31 and called at 0x557f63. The family-placement callback
(0x5585ee) bypasses the fills for an occupied lot (0x55869d, 0x558712),
and the toolbar has a separate dispatch at 0x5591e3. The run has no
instruction trace of the callback. The extra 64 draws could be two
indicator roots (0x445e4a, 0x595d65), but the caller and the exact
reload site remain untraced. The handoff models the adult cache
House 69 uses. Carrying other skeleton caches across loads needs
separate evidence.

The native pool seeding and bounded choices (0x5bc82b, 0x5bc882), the
game stream inside template scripts, names, looks and portrait writes
remain deferred. FAMs display names and Neighborhood.iff serialization
are outside the in-memory model. The two preexisting v7 FAMI records
still have their install encoding in memory. The original writer
resaves them as v9. Writing exact pool character files from native
choices still needs an instrumented recording and an estimated
600–1,200 lines.
