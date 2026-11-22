# Decision: `sims-game` Integration Wave 3 — Co-op Multiplayer Live Shell

- **Date:** 2026-11-22
- **Status:** Accepted
- **Branch:** `game/coop-multiplayer-shell`
- **Scope:** `crates/sims-game/` (plus two small `LockstepNode` helpers in `crates/sims-net/src/session/lockstep.rs` for verify-before-confirm and failed-snapshot peer disconnect, and this decision record)

## Context

With `#347` (`sims-ui::coop`), `#348` (`sims-net` deterministic lockstep + `iroh 1.5.0`), `#360` (`sims-vm::coop` multi-player simulation context, throwaway pie menu, `"SCOP"` v2 snapshots), and `#367` (`sims-game` Wave 2) all merged on `main`, the final v1 feature milestone wires live P2P co-op multiplayer directly into `sims-game` (`Game`, `NeighborhoodShell`, `Hud`, and `main.rs`) per `docs/vision.md` ("Multiplayer (v1)").

Key invariants that had to hold simultaneously:
1. **Zero UI-thread blocking:** `winit`'s event loop runs synchronously on the main thread; `iroh::Endpoint` binding, QUIC handshake, and SHA-256 install manifest hashing must never block the UI thread.
2. **Unified transport for tests and production:** The exact same `CoopController` state machine must drive both deterministic `sims_net::MemoryMesh` (for repeatable headless lockstep and drift tests) and real `sims_net::IrohTransport` (for live P2P and loopback QUIC).
3. **`Send`-compatibility of `Game` and `NeighborhoodShell`:** `main.rs` constructs `NeighborhoodShell` on a background startup thread while rendering the native `LoadingScreen` splash, so `Game` and `NeighborhoodShell` must remain `Send` with zero `unsafe`.
4. **100% bit-for-bit single-player purity:** When `coop == None`, single-player simulation (43,200-tick Goth hash) and rendering (all 8 ratchet scenes) must remain completely identical.
5. **Home-lot v1 boundaries:** Tutorial lots (`Global 26 != 0`) and away-from-home travel visits are explicitly refused with clear user-facing status messages; peers never overwrite local REDB neighborhood slots.

## Decisions

### 1. Non-blocking `IrohBridge` + Unified `CoopTransport` Enum (`crates/sims-game/src/coop.rs`)
- `IrohBridge` owns a dedicated 2-worker Tokio runtime (`sims-coop-iroh`), a bound `sims_net::IrohTransport`, and its encoded canonical `simscoop1...` `InviteTicket`.
- Async startup (`start_host_iroh_async` / `start_join_iroh_async`) spawns `build_install_manifest` + `IrohTransport::bind` onto a background worker thread and hands the ready `(IrohBridge, ManifestDigest)` back over a non-blocking `std::sync::mpsc::Receiver`, polled each wake tick on the main thread.
- Shutdown (`IrohBridge::shutdown` and `Drop`) closes the `iroh::Endpoint` asynchronously with a bounded timeout and shuts down the Tokio runtime cleanly so no threads or sockets leak on disconnect or window exit.
- `CoopTransport` unifies `Memory(MemoryTransportHandle)` and `Iroh(IrohBridge)` behind `sims_net::Transport`. `MemoryTransportHandle` stores the `!Send` `sims_net::MemoryNodeTransport` in a thread-local `BTreeMap<u64, MemoryNodeTransport>` keyed by an atomic ID, keeping `CoopTransport`, `CoopController`, `Game`, and `NeighborhoodShell` 100% `Send` with zero `unsafe`.

### 2. Mid-Game `"SCOP"` v2 Snapshot Sync, Host-Lot Restaging & Drift Resync Ordering
- Host startup primes tick 1 (`prime_tick_on_start = true`) so `LockstepNode::on_hello` always streams a verified compressed chunked `"SCOP"` v2 snapshot (`Lot::coop_snapshot_bytes` via `SnapshotEnvelope`) to every live joiner.
- `Game::advance_coop_while` interleaves host tick scheduling and stepping 1-for-1 inside the frame budget loop so the host never yields with scheduled-but-unapplied batches that could open a tick gap for a mid-game joiner.
- When `LockstepNode` emits `NodeEvent::ApplySnapshot`, `CoopController` inspects `LockstepNode::pending_snapshot_session(snapshot_id)` and runs `Lot::restore_coop_snapshot_verified(&redb_bytes, pending_session, state_fingerprint)` **first**, only calling `confirm_snapshot_applied` (which transmits `SnapshotAck(complete=true)`) after transactional VM verification succeeds. If `restore_coop_snapshot_verified` reports `SaveError::OtherHouse`, the snapshot is parked without confirming so `NeighborhoodShell::pump_coop_restage()` can seamlessly stage the host's matching local lot and complete verification, or fail cleanly with an actionable mismatch error.
- If host-side snapshot capture or encoding fails, `LockstepNode::fail_snapshot` notifies and disconnects the affected peer cleanly so joiners never stall on the sync screen.
- During active co-op, the host continues scheduling lockstep tick batches even while paused or showing a blocking dialog so unpause, Buy/Build edits, `SelectPerson`, and first-answer-wins `DialogAnswer` commands flow deterministically (`Lot::apply_coop_batch` executes commands with `stepped = false` while paused).

### 3. Per-Player Selection, Throwaway Pie Menus & Remote Indicators (`game.rs`, `menu.rs`, `hud.rs`)
- Local co-op inputs (`SelectPerson`, `SimActionPick`, `SimActionCancel`, `SetSpeed`, `SetPaused`, `SetMode`, `BuyBuildEdit`, `DialogAnswer`, `SetCameraRotation`, `SetFreeWill`) are routed exclusively through `LockstepNode::submit_local_command` and only mutate simulation state when applied in a committed `TickBatch` via `Lot::apply_coop_batch`.
- Pie menus in co-op call `menu::coop_entries`, backed by `Lot::coop_pie_menu(&Lot, player, target)` on a throwaway clone (zero mutation of shared `Lot` state before `SimActionPick` is committed).
- To display each peer's own selected Sim in the household HUD without mutating the shared `Lot::world().selected()`, `Game::draw` renders the HUD against a temporary CoW clone of `World` whenever a peer's `local_selected_sim()` differs from the host's selection, keeping `Lot::coop_world_hash()` untouched.
- Teammates' selected Sims are highlighted in-world with faceted colored diamond plumbobs (`player_badge_color(PlayerId)`) above their heads alongside `sims_ui::coop::CoopHudOverlay` and a compact `Lobby / Copy / Leave` control strip.

### 4. Shell Entry Points, Preflight Guards & CLI Flags (`hud.rs`, `neighborhood.rs`, `main.rs`)
- **Entry points:** Options → **Co-op** button (only laid out/drawn in `Mode::Options && expanded`, preserving Live Mode pixels bit-for-bit), global shortcuts `F6` and `Ctrl/Cmd+M` on both Lot and Neighborhood screens, and CLI flags `--host`, `--join <ticket>`, `--player-name <name>`, and `--coop-loopback`.
- **Preflight & Save Safety:** `Game::coop_preflight_check` refuses tutorial lots (`Global 26 != 0`) and away-from-home travel / non-residential lots cleanly. While in co-op, travel departure, Undo/Redo, and cheats are disabled; peers cannot overwrite local REDB saves or enqueue background autosaves.
- **Clean Exit:** All window exit paths (`CloseRequested`, `Escape`, Quit dialog, `leave_lot`) call `leave_coop` / `leave_coop_if_active` before exiting.

## Verification

- `crates/sims-game/tests/wave3_coop_integration.rs` tests two live `Game` instances end-to-end over both `sims_net::MemoryMesh` and real `sims_net::IrohTransport` QUIC loopback (invite ticket, `"SCOP"` v2 snapshot sync, concurrent commands, throwaway pie menu, Buy/Build placement, multi-tick `coop_world_hash` convergence, simulated drift + automatic snapshot resync recovery, manifest mismatch rejection, HUD/remote plumbob rendering, tutorial/travel preflight guards, single-player bit-exactness, and clean shutdown).
- Full release workspace test suite, 8-scene visual & determinism ratchet (`--test ratchet`), `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `scripts/check_repository.py --history HEAD`.
