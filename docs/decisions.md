# Decisions

Choices made while working toward `docs/roadmap.md`, newest first. Each entry
says what was decided, why, and what would reopen it. Entries marked
**unverified** wait on the comparison harness.

## 2026-11-13: The interpreter runs on the saved frames

An object's thread is its stack of saved frames in `World`, so the
interpreter keeps no state of its own and every snapshot holds a resumable
stack. Frames name their code owner by objt type ID, so `World` now keeps
the lot's type table (`types`, type ID to GUID; snapshot format 2).
Expression, Random Number and the operand scopes are ported from the old
prototype with its native error codes. Scopes over state `World` lacks
(tree advertisements, definition words, rooms, neighbors, jobs,
neighborhood data, type attributes) and every other primitive fail loudly.
Four semantics are **unverified**:

- New frames start with zeroed locals. The original reuses a frame arena,
  so locals inherit words left by earlier frames; saves carry no arena.
- Random Number draws from a stream the caller names. The original swaps
  the lot stream in only around the lot update (0x46d553).
- Motive reads round half to even (the x87 default); the prototype rounded
  half away from zero.
- Motive arithmetic rounds once from the x87's 64-bit significand to 32
  bits. The FPU precision the original runs it at is not known.

## 2026-11-13: Linking is checked against saved behavior signatures

Each object type in a save records two signatures: for its init and main
trees, the root's revision word shifted left 16 plus the revisions of every
tree reachable from the root (0x4026ed, recovered by the old prototype).
`sims boot` recomputes them from the linked program. Across 296 shipped
houses, 37,655 of 47,359 saved types match. In the best houses nearly all
match, and the misses differ by a revision or two, as when a later pack
revised a tree after the house was saved. A linking mistake would give
unrelated numbers.

On the Goth house only 36 of 113 types match: it was saved by the base
game, and the expansions revised most of its objects' code. The original
resets a saved object's script state when its signatures differ
(0x43a516), so booting this house exercises that reset for 77 types. The
interpreter has to implement it before the first tick.

## 2026-11-13: The Complete Collection ISO is the supported install

Decided by the maintainer. The vision named the Legacy Collection as the
primary install; it now names The Sims Complete Collection, installed from
its ISO, and leaves other editions out of scope. That is the install the
emulator image already holds and every check so far ran against, so no
code changes. It replaces the entry below on recognizing only the CD
layout: that is no longer a gap.

## 2026-11-13: The state hash is SHA-256 of the postcard encoding

`World` is encoded with postcard: fields in declaration order, integers as
little-endian varints, floats as raw bits (NaN payloads and negative zero
kept), and maps in key order. Nothing in it depends on the platform, and a
pinned-hash test runs on both CI platforms. Snapshots store the same bytes
behind a 4-byte format number, in a REDB table keyed by name. Changing the
encoding means bumping `snapshot::FORMAT` and the pinned hash. Comparison
with the original goes field by field, not through this hash: the hash only
proves that our own runs repeat.

`World` keeps every field the readers decode for objects, the simulator
and the lot arrays. It leaves out bytes nobody understands yet (format
tails, raw ObjM records), the HOUS view and model, and the wall and floor
chunks. They join when the interpreter or the comparison needs them.

## 2026-11-13: The target house is the Goth house

The boot goal needs one house. It is `House05.iff` in the base game's
shipped neighborhood: the Goth family (Mortimer, Bella and Cassandra), 245
saved objects, SIMI and ObjM version 0x3e. The installed copy is byte for
byte the shipped one, so the emulator can load it without copying files in.
The Newbie house (`House07.iff`) is smaller but was played and saved by the
old harness, so the installed copy differs from the shipped one. Reopen if
the Goth house needs something the harness cannot capture.

## 2026-11-13: Placeholder objects carry no registry word

Every ObjM record ended with a field-encoded registry word, until the
expansion houses: the `*Placeholder` object (GUID 0x3C08965F) never has one.
Across 139,000 ordinary records in the installed and shipped houses, 24
to 31 bits remain after the dynamic sprites. Across 27,000 placeholder
records, 0 to 7 remain, so no field fits. The reader now reads the word only
when at least a byte of the record remains, and stores None otherwise.
Why the original writes no word is unknown; it may skip objects missing
from the scheduler's registry. Unverified until the harness compares a
placeholder.

## 2026-11-13: Neighborhood characters register after the install

A house's Sims are defined by files in the neighborhood's `Characters`
directory. `Index::add_directory` appends that directory as one more mount,
after Downloads, so the catalog registers characters last. Where the
original registers them is unverified. A family with house number 0 has no
house: in the shipped and installed neighborhoods every such family lacks
flag bit 0, and every family with it has a nonzero house.

## 2026-11-13: Some installed houses do not load yet, on purpose

`sims houses` loads 660 of the 718 house files in the install's eight
neighborhoods and the shipped one. The 58 failures:

- 18 are `House00` and `House11` in each neighborhood: base-game templates
  with ObjM versions 0x33 and 0x38, whose record layout is not known yet.
- 40 are houses 56, 63, 69, 76 and 77, which hold Sims whose character files
  are not in the neighborhood: Superstar's community templates, and houses
  the old harness saved. The original presumably repairs or drops such residents,
  which we have not reverse engineered.

Neither blocks the target house. Reopen when a goal needs those houses.

## 2026-11-13: Later GUID registrations win; semi-globals resolve to the first match

The catalog registers object definitions in load order, and a GUID resolves
to its last registration. The old prototype recovered that rule from the
original's lookup. Multi-tile masters are the exception: the first
registered master wins, and that is for the interpreter to apply. A GLOB
link resolves to the first file of that name in load order. That part is
unverified: the Complete Collection has no duplicate semi-global names, so
nothing tells the two rules apart yet.

## 2026-11-13: Directory order is NTFS name order (unverified)

The original lists each mounted directory with FindFirstFile, and that order
decides the object catalog order. On Windows/NTFS it is name order with case
folded. `Index::build` uses that order and searches a subdirectory where its
name falls among the files. Neither detail has been confirmed against the
original's walker yet. The old harness ran the game under Wine on ext4,
whose directory order can differ from NTFS. The comparison harness must
either record the order the original saw and feed it to us, or put the
install on a file system that lists in NTFS order.

## 2026-11-13: Expansions come from the pack directories

The original reads the installed packs from the registry (`EPxInstalled`,
`SIMS_GAME_EDITION`). We have no registry, so a pack counts as installed
when its directory holds a FAR archive. On the Complete Collection install
this gives edition 0xFF, the same as its registry.

## 2026-11-13: Only the CD layout is recognized for now

The CD layout is checked against a real Complete Collection install. The
Legacy Collection's layout is not known here, and a guessed table would not
be tested by anything. It is added when someone can list a real Legacy
Collection install.

## 2026-11-13: Readers stay strict on unreferenced bad chunks

21 TTAB chunks in the original install are version 3 or zero bytes long.
Nothing references them. The reader keeps rejecting them, and loaders
decode only the chunks they use. Reopen if a referenced chunk ever fails.

## 2026-11-13: SIMI versions 0x33 and 0x38 use the later layout (unverified)

Only the template houses carry these versions. Their fields up to the budget
read plausibly with the 0x3e layout, and 0x33's unread tail is 24 bytes
shorter. Check the fields against the original when the harness loads one
of these houses.

## 2026-11-13: SPR# versions 503 and 505 share the 502/504 frame encoding

All 14 such sets in the install decode exactly with it. What the version
number itself selects is unknown.
