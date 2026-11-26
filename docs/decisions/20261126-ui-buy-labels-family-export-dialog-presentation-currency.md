# UI polish: Buy labels, Family Bin export, dialog presentation, one currency formatter

*Decided 2026-11-26.*

## Buy catalog labels

The Buy catalog's category and function buttons take their tooltips from
`sims_content::BuyLabels` (`STR#` 150 for the root row, the category's own
row inside a category). The root row of a commercial zone shows only the
columns `BuyLabels::shown` keeps, and these are the same five that
`COMMERCIAL_BITS` filters on (0, 1, 2, 3, 7). Which subcatalog a zone and
sort read (`BuyZone::catalog`) is: residential by room 0 and by function 1,
then Downtown 2, Vacation 3, Community 4, Studio Town 5 and Magic Town 6.
Item slots, page arrows and Back have no label. When the labels did not
load (no install, or the procedural art), every label is empty and nothing
is drawn. Eligibility and catalog names were already wired (#441, #452).

## Family Bin Export

The original's Family Bin has only Cancel, Create, Delete and Move In.
**Export** is a button the modern shell adds. It sits at (676, 530, 100×62)
in the native layout, uses the same art as Cancel with a plain English
label, and is enabled only while a family is selected. The Family Bin
chrome ratchet leaves its rect out of the comparison with the original.

Pressing it calls `sims_vm::caf::export_stored_family_fam` with `FINV`, the
way the original's exports are written. The file goes to:

    <saves file name>.userdata/<neighborhood id>/Export/<family name>_<id>.FAM

- **Location.** The file sits beside the shell's own `Neighborhood.iff`
  (`user_files::userdata_dir`). It is never written into the install, since
  the shell never writes there.
- **Name.** It uses the original's `<Name>_<id>.FAM` form. Characters that
  some platform cannot hold in a file name become `_`. A name whose part
  before the first dot is a Windows device name (`NUL.backup`, `COM1`,
  `COM¹`) gets a leading `_`. A name too long for a 255-byte file name is
  cut at a character to fit; the family keeps its whole name inside the
  file.
- **Overwrite.** It replaces an earlier export of the same family.
- **Atomic write.** The bytes go to a hidden temporary file beside the
  target, created fresh for that one write (never an existing file or a
  link) under a short name, so a long family name still fits. It is synced, closed and then renamed over the target, and it is
  removed if any step fails.

The Import picker scans this folder as well, under the label "Exported", so
an export can be imported back. The picker skips the reserved pool families
(`sims_vm::caf::is_reserved_pool_family`), ids past `i16::MAX`, and files
whose member list the importer would refuse: no members, more than eight,
an id past `i16::MAX`, or a repeated id. It no longer skips every id from 4000 up. A notice tells
the player the path, or the reason the export failed.

## Non-modal, anchored and large dialogs

`dialog::Presentation` records how the Dialog instruction asked for a
dialog to be shown. The legacy prototype captured the same facts
(`legacy/crates/sims-vm/src/dialog/presentation.rs`,
`legacy/crates/sims-app/src/tutorial_dialog.rs`).

| Field | Rule |
|---|---|
| modal | Operand 7, bit 7 clear. |
| large | Operand 5, bit 7. |
| anchored | The caller is not 0 and equals `World::event_owner` (0x44d906). |
| hide_title | The caller's GUID is the tutorial controller. 0xC3249A1D is a native compare constant, already cited in `sims_sim::load`. |

Layout and input rules:

- **Large.** The width search aims at 1.2 times as wide as tall instead of
  2.
- **Non-modal.** There is no button row, so the dialog is 33 + 9 shorter.
  A 12×11 dismiss box (`MinimizeBox.bmp`, UIGraphics 31, four cells in the
  same order as `WinBtn`) sits 5 in from the top right corner. It is the
  layout's only button and answers with the first button. The HUD treats a
  non-modal dialog as a dialog only where it is drawn:
  - `has_dialog` and `will_show_dialog` are false;
  - `covers` is true only inside its rect;
  - a press outside it goes on to the game;
  - a press on it goes to the dialog before an open pie menu, a held Buy
    object or a Build tool (`Game::dialog_takes`), and its release stays
    with the dialog (`Hud::take_non_modal_press`), even when it is released
    outside the window;
  - a capture whose release never came is dropped when the next press
    starts (`Hud::cancel_non_modal_press`);
  - it is drawn over the pie menu's labels, matching the order in which
    presses are routed;
  - keys never answer it;
  - the pie menu and camera strip stay open.
- **Anchored.** The dialog sits flush in the window's top right corner.
  This is the legacy placement and is **unverified** against the original.
  A modal dialog taller than the window keeps to the window's bottom,
  anchored or not, so its buttons stay on screen. A non-modal one keeps to
  the top, so its dismiss box stays on screen.
  Any other dialog is centered, modal or not. The legacy bottom-left
  placement of other non-modal dialogs came from the host and was never
  measured, so it is not ported.
- **Hidden title.** The title takes no room and is not drawn.
- **Picture icons.** Icon mode 3 still reads `BMP_` 5000 + operand 1, and
  the tutorial's pictures are 5000 to 5024.

The simulation does not yet hand non-modal dialogs to the interface.
`sims-vm`'s tick answers them at once (FIRST, or DONE for informs), so this
work is presentation and input routing that waits on the VM to stop doing
that. When the VM does stop, a non-modal prompt it publishes through
`Lot::asking` needs two things. First, `Game::has_dialog` must not count it,
since today that would swallow every press, including the dismiss box.
Second, it needs an answer path that does not require a modal `Asking`
record. The original also sets a 40-second auto-answer timer on non-anchored
dialogs that do not ask for text (legacy `widget_setting`). That timer is
not ported.

## One currency formatter

`sims_ui::currency` ports the original's locale currency formatter
(`cTSLocale`, 0x6007df), which the legacy prototype mined:

- Digits are grouped by threes, using the language's separator.
- Every language writes `$`, because the game never sets the real-currency
  byte.
- In US English a negative amount is written `-$5`. This settles the
  **unverified** note in `household.rs`.

The household and neighborhood panels both use it. The Buy popup still
writes its price as `${}`, with no thousands separator. Whether the original
groups that price is not checked yet, so moving it to the shared formatter
is a follow-up.

## What would reopen these

- A capture that shows where the original puts a non-modal dialog that is
  not anchored.
- The VM starting to show non-modal dialogs.
- A request for Export to write anywhere other than beside the saves file.
