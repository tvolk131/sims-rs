# Decisions

Choices made while working toward `docs/roadmap.md`, newest first. Each entry
says what was decided, why, and what would reopen it. Entries marked
**unverified** wait on the comparison harness.

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
