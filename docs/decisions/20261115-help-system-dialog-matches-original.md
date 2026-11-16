# The two-button "Help System" dialog matches the original's screen, apart from its corners

*Decided 2026-11-15.*

The message dialogs' layout (`20261115-message-dialog-layout.md`) was
read from the exe and checked on one one-button dialog, "Magic Town".
Track 2 has now taken a screenshot of the original's Yes/No "Help
System" dialog on the Goths' house at 800 by 600: a fresh boot, held at
lot update 2, where every Goth capture shows it. The screenshot stays in
the local cache; it is not in the repository.

- **The layout matches.** Laid out from the booted lot (the test
  `the_help_system_as_the_original_shows_it`), the dialog is at (156,
  185), 488 by 230. The title is at (404, 206), the 133-by-103 icon at
  (177, 206), the four lines at x 328 and 177 from y 251, 23 apart, and
  Yes and No at (259, 361) and (441, 361). Drawn over the screenshot,
  every part sits where the original's does: the best shift of the
  buttons and the icon is none.
- **The pixels match, as 16-bit colors.** Of the dialog's 112,240
  pixels, 111,962 are the original's once ours are cut to 5-6-5 bits
  and widened back as the screenshot widens them (below). The 278 left
  are all at or beside the four corners, which are mitred here and
  blended in the original (an approximation already recorded). The
  frame's edges, the fill, the title, the text, the icon and both
  buttons match.
- **How the screen's colors compare.** The original draws to a 16-bit
  screen. Our frame keeps the art's 8 bits a channel, so the icon and
  the buttons differ from the screenshot by a few levels until cut to
  5-6-5. The screenshot also widens 5-6-5 back to 8 bits as `v * 255 /
  31` rounded down (and `/ 63` for green), where our measured colors
  (`hud::rgb565`) repeat the high bits; the two differ by one level for
  some values, such as 12 of 31 (98 against 99). The fill and the text
  colors come out the same either way. Whether frames should be cut to
  16 bits, as the original's screen is, is left to a later change: it
  touches every panel's digest, and the render track's too.

What would reopen it: a screenshot of a three-button dialog that
disagrees, or one at another screen size.
