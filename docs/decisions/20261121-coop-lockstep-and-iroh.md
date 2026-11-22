# Co-op Lockstep Protocol, Session State, and `iroh` P2P Transport (`crates/sims-net`)

*Decided 2026-11-21.*

## Context

`docs/vision.md` ("Multiplayer (v1)") specifies co-op multiplayer on one shared
lot where every machine runs the same deterministic simulation with shared money
and clock:

- `iroh` P2P transport with invite tickets (no central game server required).
- Host-sequenced lockstep: the host stamps external commands with ticks and
  broadcasts `TickBatch`es; peers run a configurable tick buffer behind the host
  and pacing adapts to the slowest machine.
- Multi-reason pause (explicit pause, any player in Buy/Build, unanswered
  blocking dialog) and Mode Global 19 aggregation (`Build` > `Buy` > `Live`).
- Per-player simulation state (`Selected Person ID` Global 3 and
  `camera_rotation`), fixed session inputs (`Machine Level` Global 33,
  `Current Language` Global 15, `Free Will` Global 30), optional host speed lock
  with player attribution, and first-answer-wins blocking dialogs.
- Periodic state fingerprint comparison, chunked/compressed REDB snapshot
  transfer for mid-game join and drift recovery, install/custom-content file
  manifest verification, and deterministic host failover.

## Decision

1. **Standalone deterministic crate (`crates/sims-net`):**
   Keep `sims-net` independent of `sims-sim` and `sims-vm` in Part 1 so the
   protocol, session state, snapshot framing, manifest hashing, and transport
   layer compile fast and avoid coupling while `sims-vm` / `sims-sim`
   neighborhood persistence lands in parallel. Part 2 will wire `SessionState`
   into `sims-vm` (per-player Global 3 / `camera_rotation` / Global 19 /
   multi-reason pause) and `sims-game`.

2. **Strict determinism in protocol/session state:**
   `protocol`, `manifest`, `snapshot`, `session`, and `transport::MemoryMesh`
   obey the workspace determinism lints (`unsafe_code = "forbid"`,
   `disallowed_types = "deny"`, `disallowed_methods = "deny"`) with zero
   `HashMap`/`HashSet`, zero wall-clock calls, and integer-only adaptive pacing
   arithmetic. `manifest::FileManifest::from_directory` sorts `read_dir` output
   immediately and normalizes relative paths to ASCII-lowercase POSIX form so
   macOS, Linux, and Windows installs produce identical SHA-256 digests.

3. **Atomic session + REDB snapshot envelope:**
   `SnapshotEnvelope` bundles `{ session: SessionState, redb_bytes: Vec<u8> }`
   captured at `at_tick`, compresses with deflate, splits into bounded chunks
   (default 64 KiB), and verifies both compressed and uncompressed SHA-256
   digests plus bounded decompression (`take(uncompressed_len + 1)`) to prevent
   zip-bomb OOMs. For mid-game join, the host enqueues `Command::PlayerJoined`
   strictly after `at_tick` so existing peers and the joining peer apply the
   join on the exact same tick.

4. **Channel-oriented `Transport` trait with `MemoryMesh` + `IrohTransport`:**
   - `Transport` exposes non-blocking `connect`/`disconnect`/`send`/`drain_events`
     so `LockstepNode` stays synchronous and game-loop friendly.
   - `MemoryMesh` simulates a full network on a deterministic logical clock
     (round-tripping every message through length-prefixed `postcard` framing)
     with configurable latency, seeded drops, reordering jitter, partitions, and
     abrupt node crashes.
   - `IrohTransport` runs on `iroh` 1.x (resolved `1.5.0`, bumping workspace
     `rust-version` from `1.90` to `1.91`; CI runs `1.99.0`) with ALPN
     `b"sims-rs/coop/1"`, one bidirectional QUIC stream per peer (`"SIM1"`
     stream preamble + 4-byte length-prefixed `postcard` frames), deterministic
     simultaneous-dial tie-breaking, bounded inbound byte/event flow-control
     budgets, and `simscoop1...` base32 `InviteTicket` tokens.
     `IrohConfig::Loopback` uses `presets::Minimal` on `127.0.0.1:0` for
     offline/CI integration tests; `IrohConfig::P2pDefault` uses `presets::N0`
     with relay + hole punching.

5. **Deterministic host failover with epoch fencing:**
   On host departure, all surviving peers fence unexecuted old-epoch batches
   and deterministically elect the lowest remaining `PlayerId` at `epoch + 1`,
   advancing to the next surviving candidate if the primary candidate is
   unreachable. Stale batches from prior epochs are discarded. Peers take the
   same-tick continuation shortcut only when tick, adopted `SessionState`
   fingerprint, and recorded state fingerprint match; otherwise they request an
   atomic REDB snapshot (`ResyncReason::HostFailover`).

6. **Two-phase snapshot installation and resumable chunk streaming:**
   Host snapshot transmission retains a per-transfer chunk cursor and resumes
   cleanly across transport backpressure. Peers verify SHA-256 digests and keep
   stepping blocked until the caller loads the REDB world and calls
   `LockstepNode::confirm_snapshot_applied`, which atomically installs
   `SessionState` and sends the completion `SnapshotAck`.

## Reopening Criteria

- If Part 2 reveals that rollback prediction needs per-player input prediction
  windows, `LockstepNode` can be extended without changing `WireMessage` framing
  or `SessionState::apply_batch`.
- If multi-megabyte custom content packs need delta sync instead of reject-only
  manifest checks, `FileManifest::diff` already computes the exact missing and
  mismatched paths.
- If `iroh` 1.x datagrams prove measurably lower-latency for small `TickAck` /
  `Heartbeat` messages than QUIC streams, unreliable messages can be split onto
  QUIC datagrams while keeping `TickBatch` and `SnapshotChunk` on reliable
  streams.
