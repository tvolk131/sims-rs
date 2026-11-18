# The sign of the zodiac opens its help balloon

*Decided 2026-11-16.*

A press on the personality panel's sign of the zodiac opens a help
balloon over the lot: the sign's title and description, then the signs
it gets on best and worst with (`sims_ui::balloon`, `Command::Help`,
`Controls::help`, `hud::fit`). It was read from the exe and checked
against track 2's screenshot of the original with Mortimer's (Taurus)
balloon open. Every pixel of the balloon, text included, is the same in
16-bit colour. Our 8-bit art differs only below the screen's 5-6-5
precision.

- **The text** (0x519e68..0x51a00f): UIText.iff STR# 165 holds a title
  and a body for each sign, entries 2k and 2k+1 from 0. The panel
  appends to the body:
  - a line break;
  - STR# 154 entry 6 and the two signs it gets on best with, joined by
    ", ";
  - a line break;
  - entry 7 and the two signs it gets on worst with.
  The names are STR# 164 strings 1 to 12. The pairs are tables at
  0x6c5ce0 and 0x6c5d40, which are copied into `balloon.rs`; Taurus gives
  Aries, Libra and Virgo, Cancer. They are sign numbers, kept in the
  code as the signs' trait profiles are (`household.rs`). The strings
  are read from the install at run time and are never kept in the
  repo. A table that is missing or does not parse leaves its strings
  empty, and the HUD still opens. UIText.iff itself the game needs for
  its dialogs' labels, as the viewer does when a dialog is pending; with
  none pending, the viewer opens without it. The sign's name, drawn
  and sized from the panel's own list of names (`ZODIAC_NAMES`), still
  shows and takes a press, but no balloon opens.
- **The press** (0x51bfb3): the control is the sign's name, sized to it
  (0x51cd7f..0x51cdfe: x 5 and y the line plus 5 above the subpanel's
  bottom, as wide as the text and as tall as the line). `layout` cannot
  measure text, so it leaves the control empty and `hud::fit`, which the
  hosts call after `layout`, sizes it.
  - The sign is read at the press: 0x484311 matches the name the panel
    shows against the 12 signs' names and gives its number, so the
    balloon is the shown sign's. A saved 13, shown as Pisces (the name
    lookup holds it to 1 to 12, 0x4842de), opens Pisces's. The press's
    check that the number is 1 to 12 then always passes; a
    `Command::Help` for a sign outside that range does nothing here.
  - A pressed sign is presentation only: the game redraws and changes
    nothing in the lot.
  - If the panel's balloon already shows, the press hides it, whatever
    sign it shows: 0x51c025 compares the balloon object, which the panel
    has one of (+0x1c8), not the text.
  - Opening another panel hides it (0x51d375).
  - **Unverified:** closing the strip also hides it here.
  - **Not ported:**
    - the hide on a press outside a rectangle (0x51f3e5);
    - the hides on focus changes (0x51d312, 0x51d354).
- **The balloon** (0x50f6f2, 0x50f7cd, laid out by 0x50fc30):
  - It is 559 wide.
  - Its background is UIGraphics 3008, 36 by 36, cut into 12-by-12
    corners, edges and a middle (0x5df45d): the corners are drawn once,
    and the edges and middle repeated from their top left.
  - The title is in font 10 and the body in font 9, left aligned, in
    the labels' colour (0xc679).
  - The body is broken to the text's width, 530: from 5 to 535 without
    a picture.
  - The text box is the title's line plus the body's lines, at least
    103. The balloon is that plus 24, so at least 127 tall.
  - The title is at (15, 12). The body starts under the title's line
    and stops at the first line that would start below 12 above the
    bottom.
  - It sits on the subpanel, flush with its right edge. At 800 by 600
    that is x 241, with its bottom on the strip at 500. The screenshot
    agrees.
  - The original runs at 800 by 600 or more, where it always fits. In a
    smaller window it goes where the same sum puts it, partly off the
    screen. Held on the screen instead, it would cover the sign that
    closes it in a short window.
  - Without its background (3008, loaded apart from the panel's other
    art so a missing file does not stop the HUD opening), the balloon
    shows its text alone.
- **The whoosh:** "UI_whoosh" plays for the interface (object −4) when
  the balloon shows (0x50fb3f) and when it hides (0x50fbc5). A switch
  to another balloon's text plays none (0x50fbdf). The game's hud plays
  it when the open balloon comes or goes.
- **Not ported:**
  - The 250 ms fade-in and fade-out: the balloon shows and hides at
    once.
  - The pictures the needs' balloons show (an object's catalog
    thumbnail or a bitmap, in boxes at 0 and 372), and those balloons'
    text.
  - The 530 right edge of language 0x13.
  - The 50 pixels more an East Asian language takes.
  - An owner 150 tall placing it 50 lower (no live panel is that
    tall).
- **Tests:**
  - the sign texts put together from stand-in strings, and the
    compatibility tables;
  - the balloon's height from its line count, with the least height;
  - its place over the panel, and off the screen in a small window;
  - a `Command::Help` for a sign outside 1 to 12 does nothing, open or
    not;
  - the press area sized by `fit`;
  - opening and closing, by the sign and by other panels;
  - the Goths' balloon drawn from the install, with its rectangle and
    a pinned digest;
  - in the game, one whoosh for each press, no status band, and the
    balloon taking presses over the lot.
