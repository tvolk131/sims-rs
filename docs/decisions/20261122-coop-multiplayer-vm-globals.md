# Co-op Per-Player VM Globals, Camera Rotation & Command Execution (`crates/sims-vm`)

*Decided 2026-11-22.*

## Context

`docs/vision.md` ("Multiplayer (v1)") specifies co-op multiplayer on one shared
lot where every machine executes the exact same deterministic SimAntics
simulation, with per-player simulation inputs (`Selected Person ID` Global 3
and `camera_rotation`), aggregated Mode Global 19 (`Build` > `Buy` > `Live`),
multi-reason pause, fixed session globals (`Machine Level` Global 33, host
`Current Language` Global 15, shared `Free Will` Global 30), single-player-only
Tutorial / Help System suppression, throwaway per-player pie-menu availability
checks, periodic state fingerprinting, and REDB snapshot resync.

Part 1 (`crates/sims-net`, PR #348) implemented the deterministic lockstep
protocol, `SessionState`, chunked/compressed `SnapshotEnvelope`, and `iroh` P2P
transport. Part 2 wires `sims-net` into `crates/sims-vm` while preserving
100% bit-for-bit identical single-player execution (format-32 Goth day hash and
all 8 screenshot ratchet scenes).

## Decision

1. **Zero-overhead single-player isolation (`Option<CoopVmContext>`):**
   Attach an optional `CoopVmContext` to `Lot` and borrow it as
   `Option<&RefCell<CoopVmContext>>` on `Interpreter` / `Machine` (and isolate,
   compare, and keep it inside `both_ways::Cells`). When `coop` is `None`,
   every operand read/write, camera rotation read, portrait search, frame queue
   clear, removal, and reselection follows the exact single-player code path
   unchanged.

2. **Global 3 (`Selected Person ID`) multiplayer semantics:**
   - **Throwaway per-player override:** When `active_player_override` is set
     (during `Lot::coop_pie_menu` and issuer pick validation probes), Global 3
     resolves to that exact player's selected Sim.
   - ***"Am I the selected Sim?"*** (`MyData[11] == Global[3]`): When a Sim's
     own script (`me` is a person) reads Global 3 and **any** player in
     `SessionState` selects `me`, `memory.rs` returns `me as i16`. Every
     player's selected Sim behaves identically to a selected Sim in single
     player.
   - ***"Who is the player's Sim?"***: For non-Sim scripts or unselected Sims,
     Global 3 resolves to the selected Sim of the player who initiated the
     activity (`activity_owners[me]`, then `activity_owners[stack_object]`,
     then any player selecting `stack_object`), falling back to the host's
     selected Sim.
   - **Script-driven writes & host projection reconciliation** (e.g. *Magic –
     Kid To Adult Backfire*): Allowed only when `coop.is_some()` (preserving
     single-player read-only Global 3 native error `0x13`). Updates every
     player who had the affected Sim selected (`me` if person, else
     `stack_object` if person), falling back to the activity owner and finally
     the host, and **immediately reconciles `world.selected()` / plumbob
     indicators** via `Interpreter::select` so `tick.rs::end_of_update` never
     reselections against a stale marker. Likewise, `end_of_update` updates
     `host.selected_sim` only when `select_next_resident` actually changed
     selection.
   - **Frame queue clearing (`frame.rs`):** Clears `NAMED` once per Sim across
     the deduplicated sorted union of `selected_sims()` and `world.selected()`.

3. **Per-player `camera_rotation` & isolated local camera panning:**
   - `Look Towards` mode 1 (`primitives/look_towards.rs`), `Generic Sims Call
     12` (`primitives/generic.rs`), `Find Location` mode 5
     (`primitives/find_location.rs`), and head model matrix placement
     (`head.rs`) query `effective_view_rotation` / `view_rotation_for_object`.
   - For **person** scripts (`me` is a person), `resolve_camera_rotation`
     strictly delegates to `SessionState::camera_rotation_for_sim(me)` (selecting
     player's rotation with lowest `PlayerId` tie-breaker, falling back strictly
     to the host for unselected Sims — never borrowing another player's camera
     via `stack_object` or `activity_owners`). Non-person scripts check
     `stack_object` selection, then `activity_owners`, then host.
   - **Local camera panning isolation (`placement.rs`, `boot.rs`):** Local
     `Lot::view_centre` is strictly UI state and NEVER enters the co-op
     simulation (`advance_with_coop` passes `view_centre: None` and host
     rotation from `CoopVmContext`). Co-op portrait search reads an optional
     authoritative lot-coordinate `CoopVmContext::portrait_anchor` directly
     without rotation-dependent un-rotation (or uses tile-independent search
     when `None`).

4. **Authoritative co-op `Frames` schedule & Tutorial/Help suppression:**
   - `CoopVmContext::frames` (defaulting to `Frames::default()`) is installed on
     `Lot::frames` at attach, serialized inside `CoopSnapshotMeta`, restored on
     joining peers, and hashed in `Lot::coop_world_hash()`, preventing recurring
     drift after snapshot resync when peers open with differing local frame
     schedules.
   - Per `docs/vision.md` line 153 (*"Tutorial/help is single-player only"*),
     `Lot::attach_coop_context` refuses tutorial lots (`Global 26 != 0`) and
     deterministically parks non-person controllers whose linked library uses
     opcode 10 (`Tutorial`) or opcode 34 (`UI Effect`), clearing any pending
     help dialog records and `event_owner`.

5. **Sequential command execution, mode gates & Mode Global 19 (`0x4ce44f`):**
   - `Lot::apply_coop_batch` validates tick and epoch continuity (rejecting
     duplicate or out-of-order batches) and replays `batch.commands`
     **sequentially** against the pre-batch `SessionState` in exact stamped
     order so early commands never observe future commands in the same batch.
   - `SelectPerson` validates target existence before mutating `SessionState`
     so invalid selections never clobber valid selections.
   - `AnswerBlockingDialog` validates `answer.status` against
     `self.asking.shown.answers` BEFORE consuming `dialog_id` from
     `open_dialogs`, so an invalid answer leaves the dialog open for a later
     valid answer in the same batch.
   - `player_mode_to_panel_mode` maps `SessionState::effective_mode()`
     (`Build` > `Buy` > `Live`) onto the simulator's true Global 19 encoding
     (`0x4ce44f`: `0` Live, `1` Build, `2` Buy — distinct from UI panel enum
     `0x4cea5d`) and threads `CoopVmContext` through `set_mode` (`go_live` /
     `update_comeandsee`) and `sync_host_plumbob`.
   - Mode gates are enforced at each command's position in the batch:
     `SimActionPick`/`SimActionCancel` require `PanelMode::Live`; Buy edits
     require `issuer.mode == PlayerMode::Buy`; Build edits require
     `issuer.mode == PlayerMode::Build`.

6. **Frozen pre-tick dialog replay (`FrozenDialogTick`):**
   - When an unpaused step hits `StepError::Asking`, `CoopVmContext` freezes
     `FrozenDialogTick { session, activity_owners, free_will }` and retains it
     across every dialog in the interrupted tick and through
     answered-but-paused snapshots.
   - While frozen, `self.world` (including `globals[30]`, mode, pause, speed,
     and plumbob) is NOT mutated; intervening commands update live `SessionState`
     (with `SimActionPick`/`SimActionCancel`/`BuyBuild` returning
     `IgnoredWhileDialogPending`).
   - Both live replay and `restore_coop_snapshot` mid-dialog probes execute
     with the exact `FrozenDialogTick`. On tick completion, script-driven
     selection changes merge deterministically into live `SessionState` only for
     players who did not issue an intervening `SelectPerson`, deletions prune
     both `activity_owners` and live selections, and all deferred session
     changes (`FreeWill`, `Speed`, `Mode`, `Pause`, `HostCamera`, `HostPlumbob`)
     are synchronized onto the completed world.

7. **Throwaway pie-menu checks, validated picks, cancel UIDs & atomic Buy/Build:**
   - `Lot::coop_pie_menu(&self, player, target)` runs check scripts against a
     throwaway CoW `World` clone (`&self`).
   - `SimActionPick` validates both normal offers and join offers (`joining ==
     true`, probing `(args[2], sim_id)`) via a throwaway issuer-specific
     `pie_menu` probe, stages `(World, CoopVmContext)` with `activity_owners`
     set BEFORE `Interpreter::pick`, and commits ONLY on `Ok(Some(uid))` so
     failed at-once checks or full queues leak zero RNG/scratch/owner state.
   - `SimActionCancel` carries a 16-bit wire UID (`cancel_uid_to_wire`) in
     `sims-net` v1; `resolve_cancel_uid` unambiguously resolves it against the
     target Sim's active/queued `i32` UIDs (at most 9 entries), supporting
     `action_serial > 32767` cleanly while deterministically refusing any
     ambiguous 16-bit wrap collision.
   - `BuyBuild` edits execute atomically on a staged `(World, CoopVmContext)`
     with co-op callbacks enabled and catalog `BuildPrices` (loaded via
     `build_prices_from_install` or `Lot::default_coop_build_prices`), enforcing
     exact `facing`, `container`, `style`, `pattern`, `side`, and `level`, and
     purging all detached history objects inside the same staged transaction
     before committing. Any refusal or cleanup error rolls back completely.

8. **Complete fingerprints & transactional co-op snapshot resync (`"SCOP"` v1):**
   - `Lot::coop_world_hash()` hashes `World::hash()`, `Lot` step/dialog state,
     queued `Answers`, `replay_view` rotation, authoritative `Frames`,
     `portrait_anchor`, `build_prices`, `play_mode`, `play.fresh`, format-8
     auxiliary `World` tables, `activity_owners`, `next_dialog_id`,
     `active_dialog`, `FrozenDialogTick` digest, and `pending_speed_sync` under
     domain `b"sims-vm/coop-world/v1\0"`.
   - `Lot::coop_snapshot_bytes()` wraps `Saved::encode()` (keeping
     `Saved::FORMAT = 8` untouched for single-player) with `CoopSnapshotMeta`
     behind `"SCOP"` v1.
   - `Lot::restore_coop_snapshot` and `Lot::restore_coop_snapshot_verified`
     validate envelope, `Saved` bytes, dialog consistency, mid-dialog probe, and
     optional expected `StateFingerprint` transactionally, rolling back `self`
     unchanged on any error.

## Reopening Criteria

- In a future `sims-net` protocol v2 update:
  1. Widen `Command::SimActionCancel::interaction_uid` from `i16` to `i32` to
     eliminate even theoretical 65,536-wrap ambiguity.
  2. Align `PlayerMode::to_global_19()` in `crates/sims-net/src/protocol.rs`
     with `0x4ce44f` (`Live = 0`, `Build = 1`, `Buy = 2`) once callers are
     audited; `sims-vm` isolates this via `player_mode_to_panel_mode`.
  3. Expose the pending snapshot's `SessionState` on `LockstepNode` prior to
     `confirm_snapshot_applied` so callers can pass it directly to
     `Lot::restore_coop_snapshot_verified` before acknowledging the host.
- Travel destination party scripts (build-order step 3: Downtown, Vacation,
  Studio Town, Magic Town) may refine `activity_owners` with explicit party-ID
  tracking when those controllers are ported.
- When `BuyBuildEdit::SetTerrainHeight` gains a `sims-vm::build` terrain tool,
  wire it into `Lot::execute_coop_buy_build`.
