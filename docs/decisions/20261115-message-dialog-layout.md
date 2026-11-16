# Message dialogs are laid out as the original lays them out, buttons spread evenly

*Decided 2026-11-15.*

`dialog::layout` now follows the original's message dialog, read from
the exe, in place of the rules fitted to two screenshots in
`20261114-message-dialogs-text-blended-original-blends.md`; its
layout part is replaced, the rest stands. On the measured
"Magic Town" dialog it gives the same frame, title, icon, line starts
and button, and the same digest.

- **The parts** (0x524257) sit 21 in from the frame: a third of the
  frame's art (0x524a31), 12 here, and 9 more. The frame's art is not
  identified.
  - The icon is at the top left.
  - The title is 21 down. With an icon it is centered by its box over
    the text beside the icon. Without one it is centered across the
    dialog as the search's last pass left it (0x5243b5), before the
    final pass resizes it; the dialog is 300 wide before the first
    pass (0x524001).
  - The text starts 18 under the title's line, or at the top without
    a title. The dialog is the text's width and 42 wide.
  - The buttons are 18 under the text and 21 from the bottom, spread
    evenly: the text's width less the buttons', divided by one more
    than their number, before, between and after them. A button is at
    least 100 wide (0x52373c).
- **The least width** of the text is the title's box, the icon's
  width and 18 (or the icon's width, or 100), and at least 130 for each
  button (its width and 30).
- **The width search** (0x523fe3) aims at a dialog twice as wide as
  tall (0x523409). Within 0.05 of it, it is done. Otherwise the text's
  width is doubled until the dialog is at least twice as wide as tall,
  then halved from the start until it is at most that (at most 9
  times), then the two are halved toward each other while more than
  20 apart (at most 9 times). The one whose ratio is closer is kept,
  the wider on a tie. The text never goes under its least width.
- **The text box** (0x535266, 0x5351cc, drawn by 0x534fe5):
  - The icon's band is its width and 18 across, and down to 18 under
    it from the text's top, rounded to the nearest whole line (down on
    a tie).
  - Lines are fitted as the font's wrapped text fits them
    (`Font::fit`). The lines that fit down the band go beside the icon;
    the rest go under the band at the full width. One line left over
    that fits beside the icon is drawn beside it, one line on.
  - The box is the band's height and a line for each line under it.
- **The "Help System" dialog** (tick 2, Yes and No) now gets 488 by
  230, its buttons 82 apart. It is not compared with a screenshot.
- **Inferred, not read:**
  - the title's box is its text and 4 across: the title is centered
    by its box, and that is the only width that puts the measured title
    where it is;
  - where the text is first put, which the band is measured from: the
    title's box as the title was made. Here it is the title's line;
    anything from 6 less to 16 more gives the measured dialog.
- **Unverified:**
  - a button wider than 100 for a long label (the button art's width
    for its label is not read);
  - the width of a leftover line, which the original measures with an
    extra the font keeps (0x60b7e2) that is taken as 0;
  - a dialog bigger than the window: ours narrows it to the window
    (no narrower than its buttons side by side, whatever the title or
    icon), cuts lines from the end, then keeps it to the window's
    bottom, so its buttons are on screen in any window 100 a button
    wide and 54 high; it centers the title on the width it ends at;
  - a text so tall the doubling never reaches twice its height: the
    original would not stop; ours stops at 2^20 pixels.
- **Tests:** the measured "Magic Town" layout and digest; the button
  rows of two and three; the band, the lines beside and under it, and
  the line left over; the search's ratio; the title's place without an
  icon; every button on screen for long messages, tall icons and
  narrow or short windows.

What would reopen it: a screenshot of the "Help System" dialog, or of
any dialog with two or three buttons, that disagrees.
