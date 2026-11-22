# Co-op Per-Player VM Globals, Camera Rotation & Command Execution (`crates/sims-vm`)

*Decided 2026-11-22.*

## Context

`docs/vision.md` ("Multiplayer (v1)") specifies co-op multiplayer on one shared
lot where every machine executes the exact same deterministic SimAntics
simulation, with per-player simulation inputs (`Selected Person ID` Global 3
and `camera_rotation`), aggregated Mode Global 19 (`Build` > `Buy` > `Live`),
multi-reason pause, fixed session globals (`Machine Level` Global 33, host
`Current Language` Global 15, shared `Free Will` Global 30), throwaway
per-player pie-menu availability checks, periodic state fingerprinting, and
REDB snapshot resync.

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
   every operand read/write, camera rotation read, frame queue clear, and
   removal follows the exact single-player code path unchanged.

2. **Global 3 (`Selected Person ID`) multiplayer semantics:**
   - **Throwaway per-player override:** When `active_player_override` is set
     (during `Lot::coop_pie_menu`), Global 3 resolves to that exact player's
     selected Sim.
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
   - **Script-driven writes** (e.g. *Magic – Kid To Adult Backfire*): Allowed
     only when `coop.is_some()` (preserving single-player read-only Global 3
     native error `0x13`). Updates every player who had the affected Sim
     selected (`me` if person, else `stack_object` if person), falling back to
     the activity owner and finally the host, and syncs `world.globals[3]` to
     the host's selected Sim.
   - **Single-plumbob `World` invariant:** `World::selected()` and
     `world.indicators[0]` track the host's selected Sim so single-plumbob
     indicator slot invariants remain sound while `frame.rs` clears `NAMED`
     across every player's selected Sim in deterministic sorted order.

3. **Per-player `camera_rotation` across all 5 simulation readers:**
   `Look Towards` mode 1 (`primitives/look_towards.rs`), `Generic Sims Call 12`
   (`primitives/generic.rs`), `Find Location` mode 5
   (`primitives/find_location.rs`), head model matrix placement (`head.rs`),
   and dialog/UI portrait placement (`placement.rs`) query
   `effective_view_rotation` / `view_rotation_for_object`. In co-op mode this
   resolves the rotation of the player selecting `me` (lowest `PlayerId`
   tie-breaker), or for non-Sim scripts the player selecting `stack_object` or
   owning its activity, falling back to the host's rotation.

4. **Mode Global 19 encoding (`0x4ce44f` vs `0x4cea5d`):**
   In `Sims.exe` 1.91, `0x4cea5d` passes panel enum values (`Build = 0`,
   `Buy = 1`, `Live = 2`) to UI tabs, whereas `0x4ce44f` writes **Global 19**
   as `0` Live, `1` Build, `2` Buy (`buy::panel_mode`, `tick.rs:574`,
   `pick.rs:89`). `coop::player_mode_to_panel_mode` explicitly maps
   `SessionState::effective_mode()` (`Build` > `Buy` > `Live`) through
   `PanelMode` so Global 19 always carries the exact simulator encoding (`0`
   Live, `1` Build, `2` Buy) and mode transitions out of Buy/Build run
   `go_live` / `update_comeandsee` cleanly.

5. **Multi-reason pause and first-answer-wins blocking dialogs:**
   - While `SessionState::is_paused()` holds (explicit pause, any player in
     Buy/Build, or any open blocking dialog), `Lot::apply_coop_batch` syncs
     Global 17 = 1 and skips advancing simulation ticks so `world.ticks`,
     clock, motives, and `Frames` stay frozen deterministically across all
     peers.
   - When an unpaused step hits `StepError::Asking`, `Lot::apply_coop_batch`
     allocates a monotonic `DialogId` and registers it via
     `SessionState::open_blocking_dialog`. First-answer-wins ordering is
     adjudicated by `SessionState::apply_batch` (`CommandOutcome::DialogWon`);
     if a winning answer carries a button status not offered by `shown.answers`,
     all peers deterministically reject it and reopen `dialog_id` in
     `SessionState` rather than desyncing or halting.

6. **Throwaway per-player pie-menu checks (`&self`):**
   `Lot::coop_pie_menu(&self, player, target)` clones `World` via CoW, sets
   `active_player_override = Some(player)` and `player.camera_rotation`, runs
   `Interpreter::pie_menu` on the throwaway copy, and drops it. Because it
   takes `&self`, menu checks cannot mutate shared RNG or simulation state.

7. **Deterministic fingerprints and co-op snapshot resync without bumping `Saved::FORMAT`:**
   - `Lot::coop_world_hash()` hashes `World::hash()`, `Lot` step/dialog
     metadata, format-8 auxiliary `World` tables, and `CoopVmContext`
     non-session metadata (`activity_owners`, `next_dialog_id`,
     `active_dialog`) under domain `b"sims-vm/coop-world/v1\0"`.
   - `Lot::coop_state_fingerprint(session)` delegates directly to
     `session.combined_fingerprint(self.coop_world_hash())`, matching
     `LockstepNode::report_local_tick` and `LockstepNode::send_snapshot`
     bit-for-bit.
   - `Lot::coop_snapshot_bytes()` wraps `Saved::encode()` (keeping
     `Saved::FORMAT = 8` untouched for single-player) with `CoopSnapshotMeta`
     behind a `"SCOP"` v1 envelope. `Lot::restore_coop_snapshot` accepts both
     `"SCOP"` envelopes and raw `Saved` bytes, and automatically re-probes
     `Lot::asking` if the snapshot was captured while a blocking dialog was
     open.

## Reopening Criteria

- When `sims-net` is next updated, `PlayerMode::to_global_19()` in
  `crates/sims-net/src/protocol.rs` can be aligned with `0x4ce44f` (`Live = 0`,
  `Build = 1`, `Buy = 2`) once callers are audited; `sims-vm` already isolates
  this mapping through `player_mode_to_panel_mode`.
- Travel destination party scripts (build-order step 3: Downtown, Vacation,
  Studio Town, Magic Town) may refine `activity_owners` with explicit party-ID
  tracking when those controllers are ported.
- When `BuyBuildEdit::SetTerrainHeight` gains a `sims-vm::build` terrain tool,
  wire it into `Lot::execute_coop_buy_build`.
