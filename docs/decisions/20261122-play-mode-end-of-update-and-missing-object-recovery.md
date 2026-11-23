# Play-Mode End-of-Update Resilience & Missing-Object (`0x0a`) Removal Fixes

*Decided 2026-11-22.*

## Context

During an extended Play Mode (`Lot::step_play`) session on the Goth lot (`House05.iff`), the lot halted with a fatal **Lot Stopped** dialog at tick 16,407 (`world.ticks == 104850`, i.e., `88444 + 16406`):

> `The lot stopped after tick 16406: tick 16407, the lot update that starts at tick count 104850, failed: object 383: missing object (native error 0x0a)`

Two independent issues combined to produce a fatal lot stop:

1. **Why `object 383: missing object (native error 0x0a)` happened with `Fault.at == None`:**
   - On `House05.iff`, object `383` is `world.indicators[0]` (the selection plumb bob, GUID `0x2F726FFF`, created with `[383, 384]` during load).
   - `Interpreter::delete_one` (`crates/sims-vm/src/removal.rs`) omitted `Sims.exe` 1.91's `0x4481eb` head-slot release hook before walking `slots[1..]` (`0x4410e7`) to delete held children. A person deleted without going through `deselect` (such as a non-selected famous Sim holding a `STAR_MARKER`, a date partner holding `world.indicators[1]`, or a Sim whose head slot still held an indicator) caused `delete_one` to delete the indicator/marker itself. Furthermore, `Interpreter::release_from_head` (`crates/sims-vm/src/placement.rs`, `0x4481eb`) checked the *person's* GUID rather than the *head-slot target's* GUID, making its person branch dead code. Deleting an indicator also left `world.indicators` holding a dangling ID, and `Interpreter::deselect` (`crates/sims-vm/src/selection.rs`) called `take_off(world, first, true)` without checking `world.objects.contains_key(&first)` (unlike `mark_selected`), raising `Fault { object: 383, at: None, trap: Trap::native(10, "missing object") }`.
   - Independently, `forget::filter_queue` (`0x45eac7`), `people::reset_marked` (`0x45ecc2`), and `primitives::in_way::release_action` (`0x45d6c4`) called `Interpreter::callback(world, target, CANCELLED, ...)` whenever `action.target != 0` without checking `world.objects.contains_key(&target)` (unlike `drop_queued_from` at `0x45d541`), raising `Fault { object: target, at: None, trap: Trap::native(10, "missing object") }` whenever a queued action referenced an already-deleted object.
   - Multi-tile deletion (`removal::delete`), multi-tile take-off (`placement::take_off`), `take_off_one` container notification, `remove_now`, `people::release_uses`, `forget::let_go`, and dynamic-group `leave_group` (`placement/dynamic.rs`) also did not re-check `world.objects.contains_key(...)` after callbacks or child removals.

2. **Why a per-object fault outside `pass()` bypassed Play Mode and halted the entire lot:**
   - `Interpreter::run_update` (`crates/sims-vm/src/tick.rs`) only caught faults inside `Interpreter::pass()`. Everything after `pass()` ran with `?` in both Strict Mode and Play Mode:
     - `end_of_update`: `leave_group`, `remove_now`, missing object definitions, unported/non-modal dialogs, dialog-owning object removal (`world.dialogs.records`), partial-pass person removal (`+0x109` viewport-cull guardrail), and post-removal resident reselection (`select_next_resident`).
     - `daily::turn_day`, `Interpreter::day_statistics`, and `World::grow_grass` at midnight / time-of-day transitions.
   - Any `Fault` or `Trap::Unsupported` in these phases returned `Err(StepError::Tick { .. })` and set `Lot::halted = true`, permanently stopping Play Mode.

## Decision

### 1. `Sims.exe` 1.91 Parity Fixes for Stale/Missing Object References (`0x0a`)

All fixes below preserve 100% bit-exact Strict Mode behavior (verified against the 43,200-tick Goth pin `df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a`):

- **`removal::delete_one` + `placement::release_from_head` + `selection::deselect`:**
  - Fix `release_from_head` (`0x4481eb`) to inspect the head-slot target's GUID rather than the person's GUID.
  - Before `delete_one` deletes `slots[1..]` (`0x4410e7`), detach any indicator (`world.indicators` / `INDICATORS`) or pooled `STAR_MARKER` (`0xE933_7020`) in a person's head slot (`slots[2]`) via `take_off(world, held, false)`.
  - Zero matching `world.indicators` slots whenever an indicator object is removed in `delete_one`.
  - Guard `selection::deselect` with `world.objects.contains_key(&first)` / `&second` before calling `take_off`, matching `mark_selected`.
- **Stale action targets in cancellation walks (`forget.rs`, `people.rs`, `primitives/in_way.rs`):**
  - Guard `CANCELLED` callbacks in `filter_queue` (`0x45eac7`), `reset_marked` (`0x45ecc2`), and `release_action` (`0x45d6c4`) with `u16::try_from(target).ok().filter(|&t| t != 0 && world.objects.contains_key(&t))`, matching `drop_queued_from` (`0x45d541`).
- **Existence re-checks after callbacks / child removals:**
  - Re-check `world.objects.contains_key` across `remove_now`, `delete`'s multi-tile loop, `take_off`'s multi-tile loop, `take_off_one`'s container callback, `placement::dynamic::leave_group`, `forget::let_go`, and `people::release_uses`.

### 2. Play-Mode (`play: Some(&mut Play)`) Resilience in `run_update` and `end_of_update`

Strict Mode (`play: None` / `Lot::step` / `Interpreter::lot_update` / `Interpreter::end_of_update`) is **100% unchanged** — it still fails loudly on every unported or faulting case.

In Play Mode (`play: Some(&mut Play)` / `Lot::step_play`), `run_update` and `end_of_update_with` catch non-interactive faults without halting the lot:

- **Queued removals (`leave_group` / `remove_now` / missing definition):**
  - Catch any `Fault` (including `Trap::native(10, "missing object")`, `Trap::Unsupported`, `Trap::Invalid`, or script errors in `RESET`/`LEAVING`/`CANCELLED`/`DYNAMIC_GROUP` trees), record a `ScriptError` in `update.errors` (`in_thread: false`, `recovery: Recovery::Parked(None)` — preserving existing `Recovery` enum compatibility with `sims-game`), and force-finish removing the queued object via `Interpreter::force_remove_object` (plus `unlink_tile_force` for damaged dynamic groups).
  - Before calling `remove_now` in Play Mode, capture `group_parts` so if a multi-tile deletion faults partway after deleting the lead tile, all remaining partner tiles are still torn down cleanly.
  - `force_remove_object` runs zero BHAV callbacks, never deletes protected indicators or `STAR_MARKER`s held in head slots (detaching them off-lot instead), clears `PORTAL_TILE` flags, unlinks object grids (`3`/`103`) and container slots, splices `world.next_parts`, drops owned dialog records, stamps surviving route targets with `DELETED_ROUTE_TARGET`, scrubs surviving objects' slots/containers/relationships and surviving people's queues/uses/active actions, recovers any surviving person whose active action/uses referenced the removed object via `reset_person_after_purge`, removes all per-object/person side tables, and clears `play.fresh`.
- **Dialog-owning object removal (`end_of_update`):**
  - In Play Mode, drop orphaned `world.dialogs.records` with `caller == id` and continue removing `id` instead of halting.
- **Partial-pass person removal (`+0x109` guardrail in `end_of_update` and `person_tick_with_player`):**
  - In Play Mode, defer removal of queued people until the next full pass (matching `Sims.exe` `0x4412d1`) and do not fire the strict-mode `+0x109` guardrail if the queued person is visited before that full pass completes.
- **Unported / non-modal dialogs & post-removal resident reselection (`end_of_update`):**
  - Unanswered interactive dialogs and special panels still return `TickError::Unanswered` / `TickError::UnansweredSpecialPanel` so `Lot::step_play` prompts the player.
  - Unported dialog kinds / non-modal dialogs are marked `DONE` and recorded in `update.errors`; `select_next_resident` faults are caught in `update.errors` without halting.
- **Midnight rollover & grass growth (`daily::turn_day_play`, `day_statistics_play`, `grow_grass`):**
  - Per-object aging/depreciation/statistics faults, unsaved budget days, walls-only architecture counts, and grass growth failures are caught in `update.errors` with safe fallbacks so midnight rollover completes and the lot keeps ticking.

## Consequences

- Strict Mode (`Lot::step`) remains 100% bit-exact (`df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a` at Goth tick 43,200).
- Play Mode (`Lot::step_play`) never halts due to `0x0a` / `CLEANUP` / `RESET` / `LEAVE_GROUP` removal faults, `+0x109` partial-pass person removals, dialog-owning object removals, or midnight rollover faults, while preserving full `ScriptError` diagnostics in `Update::errors`.
- `Recovery` and `ScriptError` keep their exact public shape so downstream crates (`sims-game`) require zero changes.
