# Message dialogs, and text blended as the original blends it

*Decided 2026-11-14.*

`sims_ui::dialog` draws the dialog a script's Dialog instruction shows.
It reads the record in `world.dialogs`, then the strings from `STR#` 301
of the file that owns the caller's running tree. The operand meanings
and default labels come from the old prototype
(`legacy/crates/sims-vm/src/dialog.rs` and
`legacy/crates/sims-vm/src/dialog/presentation.rs`). The labels are in
`GameData/UIText.iff` `STR#` 142: OK, Cancel, Yes, No. The icon is
`BMP_` 5000 + operand 1 in the same file, when the icon mode in operand 7
is 3.

The layout was measured on two screenshots of the original at 800×600
("Magic Town" and "Studio Town", the dialogs a tick after the Goths'
house loads):

- **Placement.** The dialog is centered, rounding down.
- **Spacing.** 21 pixels between the frame and what is inside it, and
  18 between the icon, title, text and buttons.
- **Title.** In `variablesans_14`. With an icon, it is centered between
  x 141 and the right margin.
- **Text.** In `variablesans_12`, with 23-pixel lines. Lines beside the
  124-pixel icon start after it, and lines below it use the full width.
- **Buttons.** Each is 100×33, from `WinBtn` (UIGraphics 17), centered
  21 pixels from the bottom. Each half of the bitmap goes to its end of
  the button, and the middle column fills the gap. The label is
  `variablesans_12`, 5 pixels down, centered with the original's
  rounding.
- **Width.** 207, 217, 227 and so on, the first that is at least twice
  the height. This fits both screenshots (567 and 557 wide, both 276
  high) and nothing else that was tried. It is **unverified** for other
  dialogs.
- **Colors.** The text is the panel's label color. Under the pointer the
  label turns white; cells 0, 2 and 3 of `WinBtn` are the same picture.

Compared pixel by pixel after reducing both to 16-bit color: the icon,
the title and the text match on both dialogs except for 5 pixels each. The
"Magic Town" button matches exactly. On "Studio Town" the pointer was over
the button, and 24 edge pixels of its white label differ: the white blend
is **unverified**.

No bitmap in the install matches the frame, so it is drawn from
measured edge colors with mitred corners. It differs on 273 of about
11,500 frame pixels, all at the corners, where the original blends a
highlight. It is an approximation.

Text is now blended the way the original blends it, for the control
panel as well. Before, it was a plain 8-bit blend, and every edge pixel
was off. The original works in 15-bit color. Coverage level `c` (1 to
14) gives the text weight `[3, 5, 7, 9, 11, 13, 15, 18, 20, 22, 24, 26,
29, 30][c - 1]` out of 32, and what is under it gets `32 - weight`, plus
1 at odd levels. The result rounds down, and green keeps 5 bits. About
20,000 edge pixels on the dialogs' dark blue fit only that weight table.
The extra 1 at odd levels was needed only by the button labels, over a
lighter blue, so it was fitted on two backgrounds. On the control
panel's screenshot, the change fixed 1,142 text pixels and broke one; the
panel's digests are re-pinned.

**Unverified:**
- The title the original shows when the title string is empty (the old
  prototype used the object's catalog name; this draws nothing).
- `$` tokens in dialog strings, which are left as written.
- The spacing between two or three buttons.
- The icon modes other than 3.
- Text entry dialogs and the special panels; these are not drawn, and
  `Message::read` returns None for them.
- A message too long for the window. It is cut after the last line that
  leaves the buttons on screen, so the dialog can always be answered.
- The width for three buttons. The dialog is made wide enough for the
  row of buttons, in the same steps of 10.

What would reopen it: the Yes/No "Help System" dialog (tick 2) or any
other dialog screenshot that disagrees.
