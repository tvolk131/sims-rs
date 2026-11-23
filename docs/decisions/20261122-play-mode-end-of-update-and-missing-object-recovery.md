# Play-Mode End-of-Update Resilience & Missing-Object (`0x0a`) Removal Fixes

*Decided 2026-11-22.*

## Context

During an extended Play Mode (`Lot::step_play`) session on the Goth lot (`House05.iff`), the lot halted with a fatal **Lot Stopped** dialog at tick 16,407 (`world.ticks == 104850`, i.e., `88444 + 16406`):

> `The lot stopped after tick 16406: tick 16407, the lot update that starts at tick count 104850, failed: object 383: missing object (native error 0x0a)`

Two independent issues combined to produce a fatal lot stop:

1. **Why `object 383: missing object (native error 0x0a)` happened with `Fault.at == None`:**
   - On `House05.iff`, object `383` is `world.indicators[0]` (the selection plumb bob, GUID `0x2F726FFF`, created with `[383, 384]` during load).
   - `Interpreter::delete_one` (`crates/sims-vm/src/removal.rs`) did not implement `Sims.exe` 1.91's `0x447d8b -> 0x448097` indicator unregistration (`legacy/crates/sims-vm/src/indicator_unregister.rs:41..74`), so if an indicator was ever deleted (for example, deleted directly or as a held child of a non-selected person in `0x4410e7`), `world.indicators` kept its stale ID. `Interpreter::deselect` (`crates/sims-vm/src/selection.rs`) then called `take_off(world, first, true)` without checking `world.objects.contains_key(&first)` (unlike `mark_selected`), raising `Fault { object: 383, at: None, trap: Trap::native(10, "missing object") }`.
   - Note on `0x4481eb` (`Interpreter::release_from_head`, `crates/sims-vm/src/placement.rs`): matching the native transcription in `legacy/crates/sims-vm/src/indicator_pools.rs:229`, `0x4481eb` reads its *argument's* GUID before resolving a person's head slot (`slots[2]`), so passing a person directly is a no-op; callers that release a head-slot indicator (`0x447b96` `mark_famous`, `0x4476d2` `mark_selected`, `0x4467d7` `move_indicators`) pass the held object directly. That exact native behavior is preserved in Strict Mode.
   - Independently, `forget::filter_queue` (`0x45eac7`), `people::reset_marked` (`0x45ecc2`), and `primitives::in_way::release_action` (`0x45d6c4`) called `Interpreter::callback(world, target, CANCELLED, ...)` whenever `action.target != 0` without checking `world.objects.contains_key(&target)` (unlike `drop_queued_from` at `0x45d541`), raising `Fault { object: target, at: None, trap: Trap::native(10, "missing object") }` whenever a queued action referenced an already-deleted object.
   - Multi-tile deletion (`removal::delete`), multi-tile take-off (`placement::take_off`), `take_off_one` container notification, `remove_now`, `people::release_uses`, `forget::let_go`, and dynamic-group `leave_group` (`placement/dynamic.rs`) also did not re-check `world.objects.contains_key(...)` after callbacks or child removals.
   - `placement::Cursor` (`crates/sims-vm/src/placement.rs`) reset its cycle-detection set on every `advance()` call, allowing a cyclic tile link (`A <-> B` in `slot 0`) to hang `tell_neighbours` / `leave_group` infinitely instead of returning a catchable `Trap::Invalid`.
   - `placement::dynamic::unlink_tile` (`0x4228d6`) mutated `parent` words before validating the full chain, so a fault on a damaged chain (`A -> B (missing) -> C`) could corrupt `A.parent = B` before failing.

2. **Why a per-object fault outside `pass()` bypassed Play Mode and halted the entire lot:**
   - `Interpreter::run_update` (`crates/sims-vm/src/tick.rs`) only caught faults inside `Interpreter::pass()`. Everything after `pass()` ran with `?` in both Strict Mode and Play Mode:
     - `end_of_update`: `leave_group`, `remove_now`, missing object definitions, unported/non-modal dialogs, dialog-owning object removal (`world.dialogs.records`), partial-pass person removal (`+0x109` viewport-cull guardrail), and post-removal resident reselection (`select_next_resident`).
     - `daily::turn_day`, `Interpreter::day_statistics`, and `World::grow_grass` at midnight / time-of-day transitions.
   - Any `Fault` or `Trap::Unsupported` in these phases returned `Err(StepError::Tick { .. })` and set `Lot::halted = true`, permanently stopping Play Mode.

## Decision

### 1. `Sims.exe` 1.91 Parity & Safety Fixes for Stale/Missing Object References (`0x0a`)

All fixes below preserve 100% bit-exact Strict Mode behavior (verified against the 43,200-tick Goth pin `df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a`):

- **Indicator unregistration & `deselect` guard (`removal::remove_now`, `removal::delete_one`, `selection::deselect`):**
  - Implement `0x447d8b -> 0x448097` (`legacy/crates/sims-sim/src/lot_removal/immediate.rs:38`, `legacy/crates/sims-vm/src/indicator_unregister.rs:41..74`) at the entry of live `remove_now` (before physical removal/callbacks, skipped during `clear()` when the interface is switched off) and as a fallback in `delete_one`: zero matching `world.indicators` entries whenever an indicator object is removed.
  - Guard `selection::deselect` with `world.objects.contains_key(&first)` / `&second` before calling `take_off`, matching `mark_selected`.
  - Keep `placement::release_from_head` (`0x4481eb`) reading the argument's GUID before resolving a person's head slot, matching `legacy/crates/sims-vm/src/indicator_pools.rs:229`.
- **Stale action targets in cancellation walks (`forget.rs`, `people.rs`, `primitives/in_way.rs`):**
  - Guard `CANCELLED` callbacks in `filter_queue` (`0x45eac7`), `reset_marked` (`0x45ecc2`), and `release_action` (`0x45d6c4`) with `u16::try_from(target).ok().filter(|&t| t != 0 && world.objects.contains_key(&t))`, matching `drop_queued_from` (`0x45d541`).
- **Existence re-checks, route scrubbing, cycle-safe `Cursor`, and chain validation:**
  - Re-check `world.objects.contains_key` across `remove_now`, `delete`'s multi-tile loop, `take_off`'s multi-tile loop, `take_off_one`'s container callback, `placement::dynamic::leave_group`, `forget::let_go`, and `people::release_uses`.
  - Scrub deleted object IDs out of surviving route contexts (`target -> DELETED_ROUTE_TARGET`, door `word_68 -> 0`, `objects[0] -> 0`, and candidate seats `word_c` with `chosen` index adjustment) while leaving `route.path` coordinates intact so an already `UNDER_WAY` route (`0x45b2f7`) never faults on an empty path and immediate object-ID reuse never redirects a route onto an unrelated door or seat.
  - Track `visited: BTreeSet<u16>` across the full walk of `placement::Cursor` so cyclic tile links immediately return `Trap::Invalid` instead of hanging.
  - Validate the entire multipart chain (both lead and non-lead walks, including missing intermediate tiles) in `placement::dynamic::unlink_tile` before writing any `parent` word or `world.next_parts` entry.

### 2. Play-Mode (`play: Some(&mut Play)`) Resilience in `run_update` and `end_of_update`

Strict Mode (`play: None` / `Lot::step` / `Interpreter::lot_update` / `Interpreter::end_of_update`) is **100% unchanged** — it still fails loudly on every unported or faulting case.

In Play Mode (`play: Some(&mut Play)` / `Lot::step_play`), `run_update` and `end_of_update_with` catch non-interactive faults without halting the lot, reusing the existing `Recovery` enum (keeping downstream `sims-game` untouched) with a documented convention in `play.rs`:
- `Recovery::Reset`: non-terminal fallback recovery (dynamic-group forced unlink before normal removal, non-modal/unported dialog auto-answer/dismissal, selection/global-3 reconciliation, daily rollover / grass-growth fallback).
- `Recovery::Parked(None)`: queued object force-removed after a missing definition or failed `remove_now`.
- `Recovery::Parked(Some(fault))`: surviving person could not restart at `Main` during forced-removal recovery and was parked.

Specific Play Mode behaviors:
- **Queued removals (`leave_group` / `remove_now` / missing definition):**
  - Catch any `Fault` (including `Trap::native(10, "missing object")`, `Trap::Unsupported`, `Trap::Invalid`, or script errors in `RESET`/`LEAVING`/`CANCELLED`/`DYNAMIC_GROUP` trees), record a `ScriptError` in `update.errors`, and force-finish removing the queued object via `Interpreter::force_remove_object` (or `unlink_tile_force` before normal removal for `leave_group` faults).
  - `unlink_off_lot` verifies that each incoming grid head or container slot still names `id` before modifying it, so a partial `take_off_one` failure after promoting `slot 0` never erases a surviving neighbor's tile link.
  - `unlink_tile_force` repairs surviving tiles belonging to `id`'s group (either pointing to `id` or reachable via `id`'s `next_parts` chain, including non-lead removal past missing intermediates) without touching or merging unrelated multipart groups elsewhere on the lot.
  - Before calling `remove_now` in Play Mode, capture `group_parts` and record the pre-call length of the `deleted` log so if a multi-tile deletion faults partway after deleting the lead tile, remaining partner tiles are torn down cleanly while skipping any already-deleted ID (preventing deletion of a brand-new object created during a callback that reused that ID).
  - `force_remove_object` is strictly callback-free: it never deletes protected indicators or `STAR_MARKER`s held in head slots (detaching them off-lot instead), clears `PORTAL_TILE` flags, verifies and unlinks object grids (`3`/`103`) and container slots, splices `world.next_parts`, drops owned dialog records, scrubs surviving route contexts (`target`, `word_68`, `objects`, candidate `word_c`) and surviving objects' slots/containers/relationships/queues/uses/active actions, recovers affected surviving people in place without invoking callbacks (unconditionally clearing frames before `install_main`, explicitly parking the Sim and recording `Recovery::Parked(Some(fault))` if `install_main` fails), removes all per-object/person side tables, and clears `play.fresh`.
- **Orphaned & non-modal dialogs (`end_of_update`):**
  - Unanswered interactive dialogs and special panels still return `TickError::Unanswered` / `TickError::UnansweredSpecialPanel` so `Lot::step_play` prompts the player.
  - In Play Mode, orphaned dialog records whose caller no longer exists in `world.objects` are discarded before dialog dispatch and after every queued removal (catching held children, multi-tile partners, and informing dialogs posted by removal callbacks).
  - Unported dialog kinds and non-modal dialogs set `status = DONE` when `informs` is set and `status = FIRST` when `!informs` so yielding choice-dialog callers consume `FIRST` on the next tick and advance instead of looping indefinitely.
- **Partial-pass person removal (`+0x109` guardrail in `end_of_update` and `person_tick_with_player`):**
  - In Play Mode, defer removal of queued people until the next full pass (matching `Sims.exe` `0x4412d1`) and do not fire the strict-mode `+0x109` guardrail if the queued person is visited before that full pass completes.
- **Post-removal resident reselection (`select_next_resident`):**
  - Catch faults in `update.errors` (`Recovery::Reset`) and reconcile `world.globals[3]` and `coop`'s `host.selected_sim` with `selected(world)` so a fault mid-selection (such as an unported date marker in `move_indicators`) never leaves global 3 out of sync or loops on every tick.
- **Midnight rollover & grass growth (`daily::turn_day_play`, `day_statistics_play`, `grow_grass`):**
  - Per-object aging/depreciation/statistics faults, unsaved budget days, walls-only architecture counts, and grass growth failures are caught in `update.errors` (`Recovery::Reset`) with safe fallbacks so midnight rollover completes and the lot keeps ticking.

## Consequences

- Strict Mode (`Lot::step`) remains 100% bit-exact (`df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a` at Goth tick 43,200) and `0x4481eb` retains exact native argument-GUID semantics.
- Play Mode (`Lot::step_play`) never halts due to `0x0a` / `CLEANUP` / `RESET` / `LEAVE_GROUP` removal faults, cyclic tile links, damaged dynamic groups, `+0x109` partial-pass person removals, dialog-owning or non-modal dialogs, reselection faults, or midnight rollover faults, while preserving accurate `ScriptError` diagnostics in `Update::errors`.
- `Recovery` and `ScriptError` keep their exact public shape so downstream crates (`sims-game`) require zero changes.
