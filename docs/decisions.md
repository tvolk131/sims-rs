# Decisions

Choices made while working toward `docs/roadmap.md`, newest first. Each entry
says what was decided, why, and what would reopen it. Entries marked
**unverified** wait on the comparison harness.

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
