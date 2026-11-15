# The relationship panel lists who the Sim knows

*Decided 2026-11-15.*

The live panel's Relationships button now opens the selected Sim's
relationships in the subpanel (`sims_ui::hud`, `Kind::Relation`), read
from the exe; no screenshot of this panel was available.

- **The list** (0x4e9deb): every neighbor record but the Sim's own that
  the Sim has a relationship row toward, in slot order. A record stands
  for its type: its GUID gives a person (0x4e9753), and the row asked
  for is toward that person's record, the type's first (0x4ea192). So
  a second record of the same type is listed with the first's row, and
  not at all where only it has one. This is the original's behaviour,
  kept, not a slip.
  - The buttons filter it (0x51a7ea, 0x51a8c3): Family (4108) keeps the
    people with the Sim's family word (person data 61), Friends (4109)
    the mutual friends (value 0 at the friendship threshold or more
    both ways), Everyone (4110, the default) all of them. The Superstar
    button (4111) is drawn disabled, as without the expansion's flag.
  - The order (0x4e9fad): the household first, then the people on the
    lot, then by the day's relationship (row value 0), highest first.
  - `Snapshot::with_relations(world, members, friendship)` fills it.
    The host passes the friendship threshold from the tuning
    (`autonomy_constants().friendship_threshold`): 50 in the original's
    capture, though the exe's default is 25.
- **The layout** (0x51db56), on the 279 by 100 subpanel: the four 20 by
  20 buttons down the left at (3, 1), (2, 22), (2, 43) and (2, 64); the
  entries 45 by 90 from (35, 18), 45 apart, five a page; the page arrows
  (100 and 101, 9 by 49) at (24, 25) and (268, 25), each shown only
  when there is a page that way. A page past the end shows the last
  (0x51db4b).
- **An entry** (0x52838f), font 7: the face is one of five in the
  person's BMP_ 2003, by the lifetime relationship (value 2), centred
  at the top; the day's number on a dark blue box two pixels over the
  face's bottom; the day's bar 2 high and the lifetime's 4 high, 40
  wide, a fifth of a pixel a point from -100, with the dark line
  closing the filled part as the needs' meters have; the lifetime's
  number; the smiley for friends and the red heart (value 3) or else
  the pink one (value 1), 3 past the smiley's width whether the smiley
  shows or not. The bars' green and red lean from the
  panel's color by the lifetime relationship, through the original's
  16-bit blend table (0x49a2fb, 0x49a60b), ported with its float
  constants.
- **Edges kept from the original** (Sol's review asked about them):
  - The face is lifetime plus 100 over the strip's cell width (the
    bitmap's width over 5), not over a fixed 40 points
    (0x5283fa..0x528424). The install's cells are 40 wide, so the two
    agree there.
  - The dark line after a bar's filled part needs only two pixels left
    on the right (0x5287e9). An empty bar (-100 to -96) gets it a pixel
    left of the bar, where the needs' meters draw none.
- **Unverified:**
  - Font 6, which sets the buttons' and arrows' heights, is not in the
    install; its height is taken as 0.
  - Where the lifetime number's box starts (an instruction the copy
    protection hides; under the lifetime bar is taken), and so the
    icons' height.
  - That the numbers are centred, in the labels' color.
  - That an entry is lit (the purer green and red) when the pointer is
    on it.
  - Ties in the order go by the original's per-person UI index; the
    record ID stands in.
  - The original's check of who may be listed (0x423f30, and the
    on-lot person words) and its Superstar filter are not ported; at
    exactly 1024 wide it shows ten a page, not modelled.
- **Reopen** when a screenshot of the panel is available, or the hidden
  instructions are read from a clean image.
