# The job panel's left column shows the career, the pay, the performance and the friends needed

*Decided 2026-11-15.*

The job panel now draws its left column for the selected Sim
(`Kind::Work`, drawn by `hud`'s `draw_work`; the data is
`Resident::work`, which `Snapshot::with_careers` fills in), read from
the exe.

- **What it shows** (0x530501), four gadgets stacked from (3, 4) in the
  subpanel (0x52f2ff, 0x52fe37), 95 wide:
  - the career's icon, art 4902: four states across, 22 careers down,
    the row the ID of the person's career (0x4ea701; 0 with none). A row
    the art lacks draws none (0x5e3e10). Under it the level's title,
    wrapped, in font 7, or "Unemployed";
  - the salary, the level's value 17, written as the funds are, or
    "n/a", in font 8, after icon 4981 centered down the line;
  - the performance's name, or "n/a", in font 7, after icon 4982: below
    -60 Dismal, -30 Poor, 30 Average, 60 Good, else Excellent
    (0x4034fd, work.iff STR# 4096);
  - "Need" and the family friends the next level needs (its value 0),
    shown only where the family has fewer (0x4ea8d3, 0x531050), icon
    4983 after the text, 2 down.
- **Titles** come from the `STR#` of the career's ID in work.iff
  (0x403f26): level `k`'s is string `3 + 2k`, a woman's `23 + k`, which
  the panel uses where it is not empty (0x4033a4). The original skips
  the 11th career in the file's order (school), which has no `STR#`.
- **A child** (person data 58 under 18, 0x4ea908) is shown at school,
  level 0 (0x4ea722), with data 57 as the performance (0x4ea88c). The
  icon and the friends needed still follow data 56 and 57. School's
  ID is -1, so a child gets no icon and an empty title. **Unverified**
  against the original's screen; that is what the code reads.
- **The family's friends:** the lot's family (global 9) keeps a count
  that the lot works out again after each update
  (`refresh_family_panel`). `Snapshot::read` takes that count, and
  `Snapshot::with_relations` counts again where it is out of date.
- **Not drawn:** the performance's blink below 0 (0x5306df: the text
  swaps to the green at 0x6f2ab0 every 333 ms, which needs a clock the
  pure panel does not have), and the tooltips (Live.iff STR# 137).
- **Unverified:**
  - the money format (the funds' is taken; the panel uses the same
    formatter, 0x6fe90c);
  - the text color (the labels' is taken);
  - where the font's range of characters starts and ends (0x60b898):
    the original stops at a line that starts outside it; here only the
    end of the text stops it.
- **How a title wraps** (0x60b4a1, the font's wrapped text, and
  0x60b620, which fits each line; `Font::break_lines`):
  - a line is measured by its characters' advances, but the last
    character counts its glyph's width;
  - it ends after the last space or hyphen that fits, or after a line
    break. With neither, it ends mid-word, one character before the
    first that does not fit; a first character wider than the box
    takes its whole word. The spaces after a break stay on its line;
  - a line is drawn when its top is above the box's bottom (45), however
    far down it reaches, so a 13-high font shows two lines (from 22 and
    35). The wrapped text sets no clip of its own (0x60bf0c only locks
    the surfaces), and the gadget is 22 and two lines high, so the
    second line fits in it.
  - "Counter-Intelligence" breaks after its hyphen, "Medical
    Technician" after its space.
- **The original reads titles for 11, 16 or 21 careers**, by the
  game's expansion level (0x403f53). Here every career gets them; the
  full install has 21.
- **Tests:**
  - the levels, titles, children and missing careers;
  - the performance's names at their bounds;
  - the column's places, by pixel, and the line breaks, in a test font;
  - the Goths' column, with their digests pinned again, and a
    hyphenated title in the game's font.
