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
  (`0x44bf1a`, returning `Formatted { text, portrait }`) in
  `crates/sims-vm/src/primitives/dialog.rs` and wired both
  `dialog_strings` / `dialog_text_type` and `action_string` onto them
  without duplicating pass logic.
- Ported `0x4061e3` / `0x470ca7(0, 0)` / `0x471411(1, -1)` catalog name
  selection (`Machine::catalog_name`): loads the `CTSS` resource found by
  `Iff::get` (guarding `CATS` of the same ID and `unported_strings` headers
  in `Library::unported_catalog_strings`), reads US English (language 1)
  entries using `0x470ca7` + `0x471343` run rules, and selects slot 1 as
  the object's name (ignoring slot 2's description and other languages).
  Missing tables, empty slot 1, `CATS`, and unported `CTSS` headers still
  fail loudly.
- `Machine::action_string` loads strings via `Machine::string_table`
  (respecting `loaded_strings`, `unported_strings`, and `legacy_strings`)
  and formats via `Machine::format_text(..., exact_text = self.menu.is_some())`.
- Both `$TimeLocal` (`0x44c8c4`, `0x44c90b`) and `$Job*` (`0x44c966`,
  `0x44c9fc`, `0x44ca0b`) parse both `:N[:M]` numbers via `0x44caf6` and
  validate in-range local reads (`check_local`) regardless of `exact_text`.
  Out-of-range `$TimeLocal` indices or hours outside `[0, 24)` stay
  literal even when `exact_text` is true; in-range hours in `[0, 24)`
  (`0x4a6ab1`) and `$Job*` on a person fail loudly when `exact_text` is
  true.
- Classified genuine script null-dereference faults as `Trap::Invalid`
  (rather than `Trap::Unsupported`, so `sims-game`'s menu UI does not
  fall back to unchecked entries): missing string indices in
  `Add/Change Action String` (`0x4704fa` returns null, passed to `strlen`
  at `0x5b882d`) and non-person `$Job*` (`0x44ca27` reads `person + 0x60e`).
- Verified on `House05.iff`:
  - Phone object 10 (and phones 4 and 253) now list Call Cab and Services
    cleanly (`"Call Cab.../Go to Magic Town - $50"`,
    `"Call Cab.../Go to Studio Town - $50"`,
    `"Call Cab.../Explore Old Town"`,
    `"Call Cab.../Go On Vacation - $500"`,
    `"Call Cab.../Go Downtown - $50"`, `"Services.../Caterer"`).
  - Every person's pie menu on every object of the Goth house now lists
    with zero failures at ticks 0, 1500, and 6000, restoring the tick
    snapshot before each `(person, object)` pair
    (`every_person_lists_every_object_on_the_goth_house_at_0_1500_and_6000`).

## Left out / not verified

- Unported `0x44bf1a` passes (`$FamilyAssets`, `$Family`, `$Attrib`,
  `$NameAttrib`, `$NameLocal`, `$TokenNameLocal`, listed `$NeighborLocal`,
  `$CatalogLocal`, `$FameTitleLocal`, `$Grade`), exact-text formatting
  for in-range `$TimeLocal` (`0x4a6ab1`) and `$Job*`, and `0x4061e3` /
  `0x457d3b` fallbacks when US English CTSS slot 1 is empty or absent
  (`CATS`, all-language person fallback `0x470ca7(1, 0)`, OBJD name
  fallback `0x457db4`) continue to fail loudly.
- `Special Effect` (`0x44aaa6`, STR# `0x131`) still treats its formatted
  text as presentation and does not call `format_text`.
