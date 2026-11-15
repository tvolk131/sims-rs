# The job panel draws the skills and what the next level needs

*Decided 2026-11-15.*

The live panel's Job button now opens the selected Sim's skills in the
subpanel (`sims_ui::hud`, `Kind::Skills`), with the title "Job". The
panel's left column (the career's icon and title, the salary, the
performance and the friends needed) is not drawn yet.

- **Which panel:** the exe has two job panels. The one at 0x52bb86 is
  Superstar's fame panel (fame stars, fame.iff strings). The ordinary
  one is built at 0x52f2ff and drawn at 0x5301ca; this is that one.
- **Where the numbers come from** (read from the exe; no screenshot of
  this panel was available):
  - The title is font 11 at (35, 3) of the subpanel, through 0x51eaad,
    in a western language.
  - The skills are person data 10, 12, 11, 17, 18 and 15 (0x66dfec):
    Cooking, Mechanical, Charisma, Body, Logic and Creativity.
  - A skill's empty bar (4920) is at (157, 7), the next 15 below. The
    full bar (4901) is cut to 4 pixels a point, and drawn whole at 10
    points. A point is the data word over 100, clamped to 0 to 10. The
    CJK languages use 4919 and 4900 and other places; they are not
    ported.
  - A skill's label is font 7 (0x6f2ae0), right-aligned in a 62 by 16
    box at (93, 6), the next 15 below (0x6f4420). The font is now
    loaded with the others.
- **What the next level needs** (0x4ea823): the career is person data
  56 and the level 57. The career is the `CARR` of `work.iff` with
  that ID (0x404cb3). Value 1 to 6 of the level after the person's own
  is the need for Cooking, Mechanical, Charisma, Body, Logic and
  Creativity. It counts only where it is more than the person's skill.
  - `household::Careers` reads the careers.
  - `Snapshot::with_careers` fills in each resident's needs; a host
    calls it after `Snapshot::read`. The game and the viewer load the
    careers when they open and do (`sims_game::Game::snapshot` gives
    what the panel is laid out from).
  - A need is the career's whole 32-bit value, as 0x4ea883 returns it,
    so a custom career past 32,767 still marks where it says.
- **What a need draws** (0x530407), in the labels' color (0x6f2a94,
  which is the needs labels' 0xc679):
  - a line down the bar at the need's last pixel;
  - a line along the bar's bottom row, from 16 left of the bar to it;
  - the skill's label underlined. Every label is drawn; where there is
    a need, 0x5e4c01 sets the label gadget's +0x170, which 0x5e3ef1
    passes to the font as flag 4. The font's glyph drawing (0x60bf62)
    then lines each character, from where it starts to a pixel past
    its advance (glyph +0x18), in the text's color, on the row just
    under an `A` (0x60b927: `A`'s y bearing plus its height), when that
    row is inside the line. `Font::draw_underlined` does this.
- **Unverified:** the labels' vertical place in their box (centred on
  the font's line here) and the subpanel's x, as for the personality
  panel.
- **Tests:**
  - a unit test checks the layout, presses passing through, and each
    bar's fill and the need lines with stand-in art;
  - the Goths' panel is pinned by digest, gated on `SIMS_INSTALL`. Nobody
    has a skill, and Mortimer's next level (Science, "Lab Assistant")
    needs none;
  - a second digest moves Mortimer to "Field Researcher": the install's
    "Science Teacher" then needs a point of Charisma and three of Logic;
  - `sims-game`'s Goth test checks the same through the game's own
    snapshot, so a host that skips the careers fails it.

Reopen with a screenshot of the original's job panel.
