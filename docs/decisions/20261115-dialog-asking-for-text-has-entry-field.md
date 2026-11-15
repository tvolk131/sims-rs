# A dialog that asks for text has an entry field, which starts with "Hello world"

*Decided 2026-11-15.*

`sims_ui::dialog` now lays out, draws and edits the dialog a Dialog
instruction of kind 3 shows (operand 5), read from the exe. The
simulation does not run kind 3 yet: `sims-vm` stops on it as not ported,
so nothing shows one until the simulation side is done. This is the
interface's half, for that work and for track 1 to wire in.

- **What it holds** (0x436a52): the ordinary first button (as kinds 0
  and 4) and an entry field, made last (0x436c43, field ID 999). The
  field starts with the text the original passes it, "Hello world"
  (0x436c48, 0x523808): a placeholder the game ships with. Whether
  anything replaces it before the dialog shows is **unverified**; no
  screenshot of one is known.
- **Where it goes** (0x52440d): 9 under the text, at the text's left, as
  wide as the text and 36 high. The buttons go 18 under where they would
  be, and the field's height lower, so the dialog is 54 taller. The
  width search sees it the same way, since it measures the whole dialog.
- **What it takes** (0x5238a1, field +0x138, checked at 0x5f334f and
  0x5f33be): at most 64 characters; typing past them is cut. Here
  `type_into` adds a character at the end and `erase` takes the last
  one off; control characters are not typed. The original's field also
  has a caret that moves, a selection and a paste (0x5f19f5); those are
  **not ported**.
- **How it draws:** an outline, the text from its left and a caret after
  it. **An approximation:** the field's drawing is not read, and it is
  not compared with a screenshot.
- **The answer:** the host keeps the `Message` and reads its `entry` with
  the button's `AnswerDialog`. Where the original puts the text (which
  the simulation side needs) is not read here.

What would reopen it: a screenshot of a dialog that asks for text, or
the simulation side finding that the text starts otherwise.
