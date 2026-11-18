# The house panel shows Size, Yard and the house's facts

*Decided 2026-11-15.*

The live panel's House button now opens the house panel in the
subpanel (`sims_ui::hud`, `Kind::House`). It shows five ratings as bars and
four facts. It shows with or without a selected Sim, as the original's
does (0x52e49e). `sims_ui::house` works out what the panel shows, and the
hosts read it while the panel is open (`Snapshot::with_house`).

No screenshot of this panel was available, so everything here is read
from the exe.

- **Which panel:** the House button is live panel kind 4 (button +0x168).
  It is offered only on a residential lot (0x40745f). That check is not
  ported: the button is always offered. The subpanel is the class at
  0x52db8f: built at 0x52dd0e, drawn at 0x52e49e.
- **Layout**, in a western language:
  - The title "House" (Live.iff `STR#` 133, string 0) is font 11 at
    (5, 0).
  - The rating labels are font 8 (0x6f2ae4), strings 1 to 5: Size,
    Furnishings, Yard, Upkeep and Layout. Each label is sized to its text
    and moved so that its right end is 3 left of its bar (0x52e0bb). It
    sits at y 7, 25, 43, 61 and 79 (0x6f4170).
  - The bars are at (157, 6), (157, 24), (157, 42), (157, 60) and
    (157, 79) (0x6f3f30).
  - The facts are strings 6 to 9: "Sq. Ft.", "Bedrooms", "Bathrooms" and
    "Lot", followed by a value. Each is right aligned (alignment 2,
    0x5e4bf4) in an 80 by 16 box at x 0 and y 18, 38, 58 and 78
    (0x6f4008).
  - The lot's size is named by `STR#` 138: Small, Medium or Large.
  - The CJK languages use other tables, which are not ported.
- **A bar:** the rating is kept to 0 to 100 and divided by 10. The empty
  bar (4919) is drawn first. The full bar (4900, the personality panel's)
  is drawn from the same corner over it, cut to 4 pixels a tenth and at
  most its own width.
- **The facts** (0x416622), worked out at each draw:
  - The rooms are walked by ID. A room counts if it is indoors (0x46876f)
    and still marked to be counted (room +0x34, `Room::dirty`).
  - A room the last update counted and did not mark again is left out.
    That is the original's test, read as is. It looks odd. The Goths'
    rooms are all still marked after the load.
  - Each counted room adds its half tiles halved, rounded toward zero
    (0x46810a), times 9, to the square feet.
  - A counted room is a bedroom if it counted an object of category 4
    (room +0x58). It is a bathroom if it counted one of 5 to 7 (+0x5c).
    The room counts were checked against 0x467e05.
  - The lot is small under 40 tiles a side, medium under 50, otherwise
    large. The side is global 23, `World::size`; the lot pushes the same
    word into global 23 at 0x416132.
  - The original converts the square feet to square meters where the
    language measures in meters. That is not ported.
- **Size** (0x4167af) is the square feet over the number of the lot
  family's members, through a curve. With no members it is 0.
  - The lot's family (lot +0x10) is the first family whose ID is global
    9 (0x415fc4, 0x425d69).
  - The curve is string 0 of the global file's `STR#` 505, read when the
    lot is made (0x415dc0, 1-based accessor 0x4704fa). It is parsed as
    the mood curves are (0x461423) into 8 points, and weighed by 0x4167de
    (`sims_sim::motives::Curve`).
  - The result is cut toward zero (`__ftol`).
- **Yard** (0x41689a) is the outdoor rooms' summed score (the room
  manager's +0x40, `Rooms::outdoor_sum`) times the "yard score
  multiplier". It is kept to 0 to 100, with an unordered value taken as
  0, then cut toward zero.
  - The multiplier comes from the global file's tuning table 4 (FCNS
    type 0x46434e53, 0x41333a). The game reads that table when it starts
    (0x424216).
  - Names are matched ignoring case, with 0.33 as the default
    (0x666cc0).
  - The install's table has 0.33. It also has upkeep penalties of 3 for
    a dirty object and 6 for a broken one, where the exe's defaults are 6
    and 15. The original's memory image holds 0.33, 3 and 6.
- **The arithmetic** is single precision, as the original's FPU is set.
- **Not ported**, drawn as empty bars:
  - **Furnishings** is now ported: see
    `20261118-house-panel-furnishings.md`.
  - **Upkeep** is now ported: see
    `20261115-house-panel-upkeep-rating.md`. It also corrects the words
    this file guessed for the count.
  - **Layout** (lot +0x18): worked out from the lot's routing history,
    +0x3c to +0x64, filled to the "fill value for layout history".
- **Unverified:**
  - the labels' color (the needs labels' is taken);
  - how a label sits in its box down the panel. The rating labels are
    drawn at the top of their boxes. The facts are centred by the font's
    line height, where the original centres by the height of an "A".
- **Tests:**
  - Unit tests check the tuning's lookups, the facts and both ratings on
    a hand-made world (rooms that are outdoors, not marked or a negative
    size; no family; a yard out of range or unordered), and each bar's
    fill and place with stand-in art.
  - The Goths' panel is pinned by digest, gated on `SIMS_INSTALL`: 1,836
    square feet, 2 bedrooms, 1 bathroom, a large lot, Size 75 and Yard
    76.

Reopen with a screenshot of the original's house panel, and when
Furnishings, Upkeep or Layout are ported.
