# Make New Character remaining modes: negative age, zero-count cleanup, and pets

*Decided 2026-11-21.*

Ports the remaining executable-defined modes of **Make New Character** (opcode
19, `0x44ae08`), verified against the unpacked Windows Complete Collection
executable (`Sims.exe`):

## 1. Zero-count cleanup (`0x44b203` → `0x426fcd` → `0x4259a2`)

When a human's body or head count is 0 (`0x44b13f`), or a pet's body count is
0 (`0x44b051`), the original discards uncommitted body-string edits
(`0x409492(0)`), calls `0x426fcd(record)` on the neighborhood, and returns
`false` without changing `stack_object` or drawing from the game RNG:

- `0x426fcd` skips cleanup if any family references the new GUID (`0x42702c`),
  unregisters the copied definition (`0x43c9d5`, removing the entry from
  `world.characters.made`, which also frees its file number while keeping
  `next_file` advanced), and deletes the neighborhood record via `0x4259a2`.
- `0x4259a2` (`Neighbors::remove_record`) calls `0x42a8a1(id)` — getting or
  appending an empty `Inventory` entry for `id` — clears token kinds `0..=9`
  (`0x41f4e5`), vacates `slots[id - 1]` in place (`0x425a33`), and erases the
  first relationship row targeting `id` from every remaining neighbor record
  (`0x425a38` → `0x464fa2`).

## 2. Pet modes (`0x44b051`, template bytes 1 and 2)

For `species == 1` (`TemplateDog`, `0x4a70df92`) and `species == 2`
(`TemplateCat`, `0x7bea0977`) with `age >= 0`:

- After `clone_template` and the skin tone lookup (`STR# 0x2000`, native error
  `0x3b` on missing/empty), `0x44af89` forces `child = 0` regardless of `age`,
  writing `AGE = "27"`, `SKIN = tone`, `SKELETON = "dog"` or `"kat"`, and
  `GENDER = "dogmale" | "dogfemale" | "catmale" | "catfemale"` (`0x40cc2a`,
  `0x40d02f`, `0x40ce41`).
- `Wardrobe::count_pet` implements `0x408bf5(0)` → `0x409663` for gender codes
  `2..=5`: matching suits with the seven-character prefix `b???dog` or
  `b???cat`, paired with every texture starting with those seven characters
  (no skin-tone suffix; heads are not counted for pets at `0x44b01b`).
- If `bodies == 0`, runs zero-count cleanup (`0x44b203`) and returns `false`
  without modifying registry word `0x100` (`world.dialogs.declined`).
- If `bodies > 0`, reads registry word `0x100` (`0x44b064`): when `-1`, draws
  one game RNG (`0x46f457`) modulo `bodies`; otherwise uses the dialog's
  answer with 0 game RNG draws, then resets word `0x100` to `-1` (`0x44b09b`),
  sets `stack_object = record`, and returns `true`. The HUD thumbnail refresh
  (`0x4e9753`, `0x4e8c5d`) is presentation and is not modeled.

### Registry word `0x100` initialization (`0x43f72d`)

`cObjectRegistry`'s constructor initializes word `0x100` (`Dialogs::declined`)
to `-1` (`0x43f72d: or word ptr [eax + 0x100], 0xffff`), not `0`. Because `0`
is a valid pet-picker body index (`0x441d95`), `Dialogs::default()` now
initializes `declined: -1`.

- All Goth house pins (`TICK_1103` through `TICK_43200`, `b67d0c13...`) and
  Newbie pins (`NEWBIE_1802*`, `NEWBIE_4052*`) are unchanged because those
  runs already answered a two-button dialog with button 2 (`0x451a49`),
  setting `declined = -1`.
- Only `OLD_TOWN_69`, `OLD_TOWN_69_1000`, `IMPORTED_69`, and
  `IMPORTED_69_1000` (House 69 runs that only answer one-button dialogs with
  button 1) move solely due to this initial `-1` value.

## 3. Negative age (`0x44b23c`)

When `age < 0`, Make New Character clones the template selected by `species`
(`1 = TemplateDog`, `2 = TemplateCat`, else `TemplatePerson`), skips the skin
tone lookup (`0x44af49`) and `species >= 3` check (`0x44afa6`), and copies
from the running object's definition (`0x44b267`):

- Fails loudly if the running object is not a person definition (`0x457f76`
  returns null, dereferenced at `0x40934b`) or has a pet/unknown gender
  (`0x40d408` → `0x408bf5(1)` returns null, dereferenced at `0x40d42e`).
- For a human running object (`"male"` or `"female"`), copies age (`"9"` if
  `atoi(age) <= 17`, else `"27"`, `0x40cbdd` → `0x40cc2a`), skeleton
  (`"child"` / `"adult"`, skipped if the cloned template already has a pet
  gender per `0x40cee4`), skin tone (`0x40d007`, defaulting a missing string
  to `"???"`), and gender (`0x40ccac` → `0x40ce41`), with 0 game RNG draws,
  sets `stack_object = record`, and returns `true`.

## Unverified

- No native recording yet exercises negative age, zero-count cleanup, or pet
  modes of Make New Character; all three are verified against disassembly and
  covered by unit and interpreter tests.
- Presentation-only derived outfit strings (`0x409315`, `0x40a588`) and
  chosen body/head strings (`STR# 200` rows 1 and 2) remain unmodeled, as in
  mode 0.
