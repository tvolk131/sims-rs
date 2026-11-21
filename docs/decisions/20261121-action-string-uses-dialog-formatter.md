# Add/Change Action String uses the shared `0x44bf1a` string formatter

*Decided 2026-11-21.*

Closes the third and final Goth house pie-listing failure from
`20261120-check-tree-faults-and-failed-play-steps.md` (Call Cab's check
tree `0x2074` on phones 4, 10, and 253 formatting `$$Local:1` fares).

## What the original does

`Add/Change Action String` (opcode 50, `0x44cc07`):

1. Loads string table `word 0` from the running frame's file (`0x416920`)
   with `0x47028b(file, table, 0)` — the same `cTSStringSet` loader that
   dialogs (`0x43601c`) use — and fetches 1-based string `byte 4` via
   `0x4704fa(index, -1)`.
2. Zero-initializes four action-argument words (`args = [0; 4]`) and calls
   the shared string formatter `0x44bf1a(&text, frame, &args, &icon_type)`
   before checking whether a pie menu is being built (`0x45876b`) or
   whether `0x6c4d6c` is set for a thread's running interaction.
3. Inside `0x44bf1a`:
   - Pass order matches `#304`: `$FamilyAssets` / `$Family`, `$Me`,
     `$Object`, `$Local`, `$Attrib` / `$NameAttrib` / `$NameLocal` /
     `$TokenNameLocal`, `$NeighborLocal`, `$Neighbor`, `$CatalogLocal` /
     `$FameTitleLocal` / `$Grade`, `$TimeLocal`, and `$JobOffer` /
     `$JobDesc` / `$Job`.
   - Only `$Family` (`0x44c0e3`, writing `args[1] = family_id`) and
     `$Neighbor` (`0x44c6c7`, writing `args[0] = stack_object` when
     `0x425afe(stack_object) != 0`) write into `args`.
4. When a pie menu is being built (`0x45876b`), if `args[0] == 0`, sets
   `args[0] = frame->stack_object` (`0x44cca3`), then adds the menu entry
   (`0x458789`) with the formatted text and `args`. (Since `$Family` is
   not ported and `$Neighbor` writes `frame->stack_object` into `args[0]`,
   every ported path leaves `args == [frame.stack_object, 0, 0, 0]`.)

## What changes here

- Extracted `Machine::string_table` (`0x47028b`) and `Machine::format_text`
  (`0x44bf1a`, returning `Formatted { texts, types }`) in
  `crates/sims-vm/src/primitives/dialog.rs` and wired both
  `dialog_strings` / `dialog_text_type` and `action_string` onto them
  without duplicating pass logic.
- `Machine::action_string` now loads strings via `Machine::string_table`
  (respecting `loaded_strings`, `unported_strings`, and `legacy_strings`)
  and formats via `Machine::format_text(..., exact_text = self.menu.is_some())`.
- When `exact_text` is true (building a pie menu label):
  - `$Local` replaces `$Local[:N[:M]]` with `locals[N]` in decimal while
    leaving literal `$` characters intact (`$$Local:1` with `local[1] = 50`
    formats to `$50`).
  - Single-name `$Me`, `$Object`, and `$Neighbor` substitute cleanly;
    multi-entry catalog tables where `0x4061e3` could pick among distinct
    names fail loudly.
  - Tokens whose output text is not substituted (`$TimeLocal`, `$JobOffer`,
    `$JobDesc`, `$Job`) fail loudly instead of returning unformatted text.
- When `exact_text` is false (thread run or check outside a listing, where
  the text is dropped), `format_text` enforces the same fault/termination
  checks as dialogs (`dialog.rs`).
- Verified on `House05.iff`:
  - Phone object 10 (and phones 4 and 253) now list Call Cab and Services
    cleanly (`"Call Cab.../Go to Magic Town - $50"`,
    `"Call Cab.../Go to Studio Town - $50"`,
    `"Call Cab.../Explore Old Town"`,
    `"Call Cab.../Go On Vacation - $500"`,
    `"Call Cab.../Go Downtown - $50"`, `"Services.../Caterer"`).
  - Every person's pie menu on every object of the Goth house now lists
    with zero failures at ticks 0, 1500, and 6000
    (`every_person_lists_every_object_on_the_goth_house_at_0_1500_and_6000`).

## Left out / not verified

- Unported `0x44bf1a` passes (`$FamilyAssets`, `$Family`, `$Attrib`,
  `$NameAttrib`, `$NameLocal`, `$TokenNameLocal`, listed `$NeighborLocal`,
  `$CatalogLocal`, `$FameTitleLocal`, `$Grade`) and exact-text formatting
  for `$TimeLocal`, `$Job*`, and multi-entry CTSS lookup (`0x4061e3`)
  continue to fail loudly.
- `Special Effect` (`0x44aaa6`, STR# `0x131`) still treats its formatted
  text as presentation and does not call `format_text`.
