# Special `Dialog` kinds (`5..=15`) and Complete Collection interactive object sweep

*Decided 2026-11-22.*

## Context

Live playtesting of `main` (`b4973e65`) on Goth (`House05`) halted when clicking
the Magic Town Hole (`ClownPortal.iff`, tree `0x1001` node `2`) with:

`object 21, tree 0x1001 node 2: unsupported: Dialog kind 13 (operand 5) is one of the game's special panels, which is not ported (0x451552)`

Previously, `crates/sims-vm/src/primitives/dialog.rs` only ported ordinary
modal dialogs (kinds `0..=4`) and trapped on all eleven special panel kinds
`5..=15` (`0x4515ae..0x4519ad`).

## What the original does (`Sims.exe` 1.91)

### 1. `Dialog` primitive (`0x451552..0x451ab4`)

Before switching on `kind = operand[5] & 0x7f` (`0x4515ae..0x4515b1`), `Dialog`
runs the common pre-check (`0x451562..0x45158d`): if the running object already
owns a dialog record (`0x44435c`) whose status is `1` (shown, unanswered), it
closes it (`0x437058`, setting status `5`). Next, if `informs` (operand `7` bit `0`, `0x451593..0x4515a9`) is set, it routes
through `0x4443ab` and takes the True branch immediately; `0x4443ab` has
dedicated neighborhood-picker constructors for kinds `5, 7, 8, 11, 13` (passing
literal `0`, ignoring operand `7` bit `7` and skipping `0x43601c` / portrait
formatting; kind `13`'s constructor `0x551285` initializes `[dialog + 0xec] = 1`,
i.e., `via_blimp: true`) in addition to ordinary kinds `0..=4`. Blocking special kinds
`5..=15` never create a dialog record; instead they communicate through
`cObjectRegistry` (`reg`) slots read by `end_of_update` (`0x4413e1..0x442320`),
except kinds `12` and `15` which open synchronously inline inside `Dialog`.

| Kind | Registry slots (`reg+...`) | `Dialog` (`0x451552`) | `end_of_update` (`0x4412d1`) | Modal & result contract |
|---:|---|---|---|---|
| **5** | `+0xea` (`i16`) | `0x4515b3`: `0 -> -1` + Yield; `> 0 -> temp[0] = v`, clear to `0`, True | `0x4414cd` -> `0x549dae`: Downtown lot picker (`21..=30`) | `Lot(Some(21..=30))` stores lot; invalid/cancel resets `0` |
| **6** | `+0xf4` (`i16`), `+0xf6` (`i16`), `+0xf8` (`*Person`) | `0x451657`: caller must be person (`0x1c`); if `+0xf8 == null`, sets `+0xf4 = temp[0]` (mode), `+0xf8 = me` + Yield; else clears `+0xf4` & `+0xf8`, writes `temp[0] = +0xf6`, returns `+0xf4 == -2` | `0x4417b3` -> `0x508f22`: Wardrobe / Clothing Shop (`0x5097c9` maps mode `0..=5`) | `0x5098bd`/`0x5098db`: accept packs `(idx << 16) \| 0xfffe` (`+0xf6 = idx`, `+0xf4 = -2`), cancel `0xffff` (`+0xf4 = -1`) |
| **7** | `+0xec` (`i16`) | `0x4515f0`: `0 -> -1` + Yield; `> 0 -> temp[0] = v`, clear to `0`, True | `0x441df2` -> `0x5686e9`: Vacation Island lot picker (`40..=48`); `on_residential` via `0x40745f` | `Lot(Some(40..=48))` stores lot; invalid/cancel resets `0` |
| **8** | `+0xf2` (`i16`) | `0x4515df`: `0 -> -1` + Yield; `> 0 -> temp[0] = v`, clear to `0`, True | `0x4415fd` -> `0x556721`/`0x4425a8`: Old Town / Community lot picker; `unleashed = (expansions & 0x20) != 0`; validates `zoning(lot) == 2` (`0x42b9e3`) | Valid community lot stores lot; non-community/cancel resets `0` |
| **9** | `+0x100` (`world.dialogs.declined`), `+0x104` (`*Person`), `+0x108` (`i16`) | `0x45181c`: `stack_object` must be on-lot person (`0x1c`); if `+0x104 == null`, sets `+0x104 = pet` + Yield; else clears `+0x104`, writes `temp[0] = +0x100`; accept (`+0x108 == -2`) leaves `+0x100` intact for `Make New Character` (`0x44b064`) + True, cancel sets `+0x100 = -1` + False | `0x441b8b` -> `0x521788`: Pet Adoption / Buy Pet picker (`DisplayFloorPetPen.iff` `0x1011`), saves `world.speed()`, sets `760` (`0x441bb2..0x441bd7`), clears `+0xf6 = 0` (`0x441d52`), restores saved speed via `World::set_speed` (`0x441de8`) | Accept writes `+0x100 = idx`, `+0x108 = -2`; cancel writes `+0x100 = idx`, `+0x108 = -1` |
| **10** | `+0xe8` (`i16`) | `0x45179a`: caller must be person (`0x1c`); `0 -> -person.data[31]` + Yield; `-9999` (`0xd8f1`) clears to `0`, leaves `temp[0]` untouched, False; `> 0 -> temp[0] = v`, clear to `0`, True | `0x4413e1` -> `0x5221d0`: Phone / Call Neighbor picker (`PhoneGlobals.iff` `0x2085`) | `Neighbor(Some(id > 0))` stores `id`; cancel stores `-9999` (`0xd8f1`) |
| **11** | `+0xee` (`i16`), `+0x10c` (`*Person`) | `0x451601`: `0 -> -1` + Yield; `> 0 -> temp[0] = v`, clear to `0`, True | `0x441f72` -> `0x5644b5`: Studio Town lot picker (`81..=89`); checks `+0x10c` (set by `Generic Sims Call 35` at `0x451142`) for `person.data[80] > 0 && person.data[56] == 0` (`star_limo`) and clears `+0x10c = null` on valid pick (`0x442152`) | `Lot(Some(81..=89))` stores lot; invalid/cancel resets `0` |
| **12** | none (inline) | `0x4518e2` -> `0x5107dc(dynamic_cast<cXPerson*>(me), 0)`: passes `NULL` when `me` is not a person (no `0x1c` throw); opens Spellbook reference modal synchronously inline inside `Dialog` (no yield, same tick) and returns True | none | `Dismissed` |
| **13** | `+0xf0` (`i16`), `[0x6f2a58]+0xf8` | `0x451612`: when `0`, yields without arming `-1` if `[0x6f2a58]+0xf8` (`magic_picker_blocked`) is set (`0x451628..0x451642`), else `0 -> -1` + Yield; `> 0 -> temp[0] = v`, clear to `0`, True | `0x44216e` -> `0x551285`: Magic Town lot picker (`90..=98`); writes `globals[34] != 0` to `[dialog + 0xec]` (`0x4421e3..0x442219`, read at `0x5545ee`: `false` for Magic Hole `ClownPortal.iff` node 64, `true` for Magic Blimp `phonepluginmagiclots.iff` node 98) | `Lot(Some(90..=98))` stores lot; invalid/cancel resets `0` |
| **14** | `+0xf4` (`i16`), `+0xf6` (`i16`), `+0xfc` (`*Person`) | `0x4516f9`: caller must be person (`0x1c`); if `+0xfc == null`, sets `+0xf6 = temp[0]`, `+0xfc = me` + Yield; else clears `+0xf6` & `+0xfc`, writes `temp[0] = +0xf6`, returns `+0xf4 == -2` (`Charms.iff` `0x1017`, `CharmsKid.iff` `0x100d`) | `0x4419a6` -> `0x536a6b`: Magic Charm Transform picker (`0x5373ee`/`0x53740b`) | Accept sets `+0xf6 = idx`, `+0xf4 = -2`; cancel sets `+0xf6 = idx`, `+0xf4 = -1` |
| **15** | none (inline) | `0x451946` -> `0x5107dc(dynamic_cast<cXPerson*>(me), 1)`: passes `NULL` when `me` is not a person (no `0x1c` throw); opens Cookbook reference modal synchronously inline inside `Dialog` (no yield, same tick) and returns True | none | `Dismissed` |

### 2. `end_of_update` dispatch order (`0x4413e1..0x442320`)

In `cObjectRegistry::EndOfUpdate`, Kind 10 (`+0xe8 < 0`) runs first and falls
through to Kind 5 (`+0xea < 0`), which falls through to a **mutually exclusive**
`if / else if` chain in exact order:
`Kind 8 -> Kind 6 -> Kind 14 -> Kind 9 -> Kind 7 -> Kind 11 -> Kind 13`
(where the first matching branch jumps to `0x442320` on completion).

### 3. Paired `Generic Sims Call` routines

- **Routine 35 (`0x451142..0x4511a1`)**: casts `me` to `cXPerson` (native error
  `0x1c` if not a person) and stores `me` in `reg+0x10c` (`studio_caller`), read
  by Kind 11 (`0x441f94`) to determine whether the Studio Town limo is offered.
- **Routine 14 (`0x44fe22..0x44fe37`)**: writes `temp[0]` (`object +0x3a`) into
  global `31` (`0x1f`, via `0x46d9c4`, used by stereos/boom boxes to track the
  active station) and takes the True branch. Surfaced by the Complete Collection
  sweep on Newbies (`House07`) `"Down Wit Dat" Boom Box`.

## Decision

1. **Typed special-panel protocol on `sims-vm`:**
   - Added `SpecialPanels`, `SpecialPanelRequest`, `SpecialPanelResponse`,
     `ShownSpecialPanel`, `ScriptedPlayer`, `AskingSpecialPanel`, and
     `SpecialPanelAnswerError`.
   - Kept `StepError`, `Answer`, `Answers`, and `Asking` unchanged so `sims-game`
     and `sims-render` compile with zero cross-crate churn; `Lot::step` /
     `Lot::step_play` return `Err(StepError::Asking { tick })` and callers
     inspect `Lot::asking()` (ordinary kinds `0..=4`) or
     `Lot::asking_special_panel()` (kinds `5..=15`) and reply via
     `Lot::answer_special_panel(...)` (or pre-seed headless replays via
     `Lot::queue_special_panel_responses(...)`).
   - Inline modals (kinds 12 & 15) invoke `Player::special_panel` synchronously
     inside `Dialog` via `InlineSpecialPanel<'a>` so accepted dismissals return
     `Flow::Branch(true)` immediately with the exact remaining `TICK_BUDGET`
     (including inside nested `Run Tree By Name` mode 0/1 callbacks), while
     unanswered interactive waits unwind cleanly before play-mode recovery and
     headless `Player` implementations auto-dismiss books without consuming
     queued picker responses.
   - `build.rs` and `travel.rs` gate mutations/departures/returns on the shared
     pending-input guard so the pre-tick world is preserved for replay.
   - Co-op (`coop.rs`) freezes and restores special panels alongside ordinary
     dialogs (`CoopVmContext::active_special_panel`,
     `CoopBatchReport::opened_special_panel`, `COOP_SNAPSHOT_VERSION = 3`,
     full request hashing in `CoopWorldFingerprintTail`, and lockstep answering
     via `special_panel_answer_command` over `Command::AnswerBlockingDialog`).
   - `both_ways::Cells` isolates and compares `special_panels`.
2. **Backwards-compatible save schema (`Saved::FORMAT = 9`):**
   - Added `SavedV8` and `migrate_v8_to_v9` to `SAVED_MIGRATIONS` (`6 -> 7 -> 8 -> 9`),
     persisting `special_panels` and `special_panel_responses`. Existing `v6`, `v7`,
     and `v8` saves migrate deterministically with zero regression on the
     43,200-tick Goth day hash.
3. **Automated Complete Collection interactive object sweep (`crates/sims-vm/tests/interaction_sweep.rs`):**
   - Added a live-install test for the Goth Magic Hole (`MagicStarterKit` ->
     `Open` -> `ClownPortal` -> `Jump In` -> `Dialog kind 13` + mid-panel
     save/restore) and an `#[ignore]` 9-lot sweep across base + all expansions
     (`House05`, `House07`, `House69`, `House21`, `House40`, `House58`,
     `House81`, `House90`, `House93`) writing a ranked `Trap::Unsupported`
     report when `SIMS_SWEEP_REPORT` is set.

## What would reopen this

- A comparison capture against `Sims.exe` showing side effects inside one of the
  modal UI constructors (`0x508f22`, `0x521788`, `0x5221d0`, `0x536a6b`,
  `0x549dae`, `0x551285`, `0x556721`, `0x5644b5`, `0x5686e9`, `0x5107dc`) beyond
  the registry slot writes modeled here (**unverified** by live trace; read from
  disassembly).
- Follow-up wiring in `sims-game` (Track 1) connecting both single-player
  `Lot::asking_special_panel()` -> `Lot::answer_special_panel(...)` and co-op
  `CoopBatchReport::opened_special_panel` -> `special_panel_answer_command(...)`
  (at `crates/sims-game/src/game.rs:3942` and `:7970`) to
  `sims_vm::travel::depart_to_destination`, phone neighbor picker UI, wardrobe /
  shop / pet / charm pickers, and spellbook / cookbook overlays.
