# REDB Schema Migrations & Crash-Safe Rotating Autosave Backend

*Decided 2026-11-21.*

Implements the REDB persistence durability, versioning, and autosave guarantees
from `docs/vision.md` (**Saves**) inside `crates/sims-sim` (`sims-sim::migration`,
`sims-sim::autosave`, and `sims-sim::neighborhood_store`).

## Forward Schema Migrations (`sims-sim::migration`)

`StoredNeighborhood` records in `NEIGHBORHOODS` carry a 4-byte little-endian
schema version header (`NEIGHBORHOOD_MIN_FORMAT = 1`, `NEIGHBORHOOD_FORMAT = 3`)
followed by a `postcard` body. Instead of rejecting older versions outright,
`NEIGHBORHOOD_MIGRATIONS` registers an explicit, deterministic forward chain
(`v -> v + 1`):

- **`v1 -> v2`**: decodes `StoredNeighborhoodV1` strictly via
  `decode_postcard_exact` (rejecting truncated or trailing bytes), initializes
  `deleted_characters` to empty, and derives a deterministic non-zero 128-bit
  `instance_id` via
  `SHA-256("sims-sim:neighborhood-legacy-instance-id:v1" || (record.len() as u64).to_le_bytes() || record || (body.len() as u64).to_le_bytes() || body)`.
  Also accepts the historical commit `0c25ad21` layout (version `1` header
  carrying `deleted_characters` and `instance_id`), which is strictly disjoint
  from pure `v1` under exact postcard decoding, preserving fields already
  present and normalizing a zero `instance_id` deterministically.
- **`v2 -> v3`**: decodes `StoredNeighborhoodV2` strictly, preserves non-zero
  `instance_id` (normalizing `0` deterministically), and backfills
  `deletion_epoch = 1` and `deleted_at_epoch[guid] = 1` whenever a `v2` record
  carries non-empty `deleted_characters` so pre-migration saved lots
  (`deletion_epoch == 0`) continue stripping stale character instances if a
  deleted GUID is later reused. Also accepts the historical commit `1719402e`
  layout (version `2` header carrying `deletion_epoch` and `deleted_at_epoch`),
  strictly disjoint from pure `v2` under exact postcard decoding, preserving
  existing epoch fields verbatim.
- **Current-version validation**: every record passing through `migrate_bytes`
  (including `v3` records) is strictly validated with `decode_postcard_exact`
  so truncated payloads, corrupt bytes, or trailing bytes fail cleanly.
  Unframed `v1..=v3` neighborhood rows carry no outer checksum and therefore
  receive structural and exact-length validation only (a future
  `NEIGHBORHOOD_FORMAT = 4` can add an outer key+version+body SHA-256 envelope
  for bit-flip detection on structurally valid mutations).

### Determinism & Atomicity

- Because `derive_legacy_instance_id` is a pure function of `(record_name, body)`,
  read-time migration (`StoredNeighborhood::decode` / `decode_with_report`) and
  in-DB migration (`NeighborhoodStore::migrate_all` / `migrate_neighborhood`)
  always agree bit-for-bit on `(instance_id, revision)`.
- `NeighborhoodStore::migrate_all` validates and migrates every row in memory
  inside a single REDB write transaction before writing any upgraded row or
  recording `NEIGHBORHOOD_META["schema_version"]`. If any record has a
  truncated header, unsupported version (`0` or `> NEIGHBORHOOD_FORMAT`), or
  corrupt payload, the transaction aborts with zero partial mutations on disk.
- `NeighborhoodStore::save_neighborhood` and unchecked
  `save_lot_and_neighborhood_checked` propagate decode errors on existing rows
  rather than silently overwriting corrupt or future-version records.

## Crash-Safe Rotating Autosave Store (`sims-sim::autosave`)

`AutosaveStore` adds three REDB tables that can share `NeighborhoodStore`'s
`Arc<Database>` handle (`NeighborhoodStore::autosaves` /
`autosaves_with_policy`) or open a standalone REDB file (`AutosaveStore::open`):

- `autosave_manual` (`(&str, u32) -> &[u8]`): single manual save slot per
  `(neighborhood_id, lot_id)`.
- `autosave_ring` (`(&str, u32, u64) -> &[u8]`): rotating autosave ring keyed
  by `(neighborhood_id, lot_id, autosave_seq)` with configurable
  `AutosavePolicy::max_autosaves` (default `5`, minimum `1`).
- `autosave_seq` (`(&str, u32) -> u64`): strictly monotonic per-lot sequence
  counter (`max(stored_seq, max_existing_ring_seq) + 1`, never wrapping or
  reusing deleted sequence numbers).

### Record Envelope & Integrity

Each manual or autosave record is framed as:

- `b"SAVG"` (4-byte magic)
- `version: u32` (little-endian, `AUTOSAVE_FORMAT = 1`)
- `record_sha256: [u8; 32]` (`SHA-256("sims-sim:autosave-record:v1" || version || body)`)
- `body` (`postcard`-encoded `AutosaveBodyV1` carrying `slot`,
  `neighborhood_id`, `lot_id`, `sim_tick`, optional caller-supplied
  `saved_at_unix_secs`, `label`, `payload_kind`, `payload_sha256`, and
  `payload` bytes).

Version validation runs before checksum verification so records from newer
builds report `SaveError::UnsupportedVersion` rather than `CorruptChecksum`.
Decode also verifies `payload_sha256` and record-to-key binding
(`neighborhood_id`, `lot_id`, `slot`).

### Non-Destructive Retention & Recovery

- **Atomic write & prune:** `commit_job` encodes `World` and hashes the payload
  outside the REDB write transaction, then inserts the new monotonic autosave
  slot, advances `autosave_seq`, and prunes only oldest *intact* slots beyond
  `max_autosaves` in one atomic REDB write transaction. Manual saves and
  corrupt/unsupported-version autosave slots are never modified or deleted by
  autosave writes or pruning.
- **Corrupt manual protection:** `SaveTarget::Manual` validates any existing
  manual record first and refuses to silently overwrite a corrupt or
  unsupported-version manual save unless the caller explicitly commits
  `SaveTarget::ManualForceReplace`.
- **Listing & streaming fallback:** `list_saves` returns
  `[Manual?, Autosave(newest)..Autosave(oldest)]` with corrupt slots reported
  inline as `SaveSlotEntry::Corrupt`. `load_latest_intact_autosave` and
  `load_with_fallback` execute in a single MVCC REDB read transaction, looking
  up `preferred` directly by key first and streaming fallback candidates lazily
  without copying the full ring up front, returning the newest intact candidate
  plus all skipped corrupt slots untouched on disk.
- **Payload validation:** Built-in validation decodes `WorldSnapshot` payloads
  with `World::decode` on write, load, list, prune, and fallback; callers
  storing external formats (`SavedLot`, `Custom`) can pass a validator via
  `commit_job_validated`, `load_save_validated`, `list_saves_validated`,
  `load_latest_intact_autosave_validated`, and `load_with_fallback_validated`
  so malformed external payloads are rejected before writing and never evict or
  mask usable predecessors.
- **Neighborhood lifetime fencing:** Shared handles obtained from
  `NeighborhoodStore::autosaves` / `autosaves_with_policy` require every
  `SaveJob` to specify `expected_instance_id` (`SaveJob::autosave_for_instance`,
  `manual_for_instance`, or `.with_expected_instance_id`), verified inside the
  REDB write transaction against `NEIGHBORHOODS[neighborhood_id]` before any
  slot mutation. Queued background jobs that outlive a neighborhood deletion or
  re-import abort cleanly with `SaveError::StaleNeighborhoodInstance`.
- **Background worker ergonomics:** `AutosaveStore` is `Clone + Send + Sync`
  and `SaveJob` / `SavePayload` (`World(Box<World>)`, `SavedLot(Vec<u8>)`,
  `Custom`) are fully owned (`Send + 'static`), allowing the main thread to
  clone a CoW `World` (~4.25 µs) and commit on a worker thread without
  wall-clock reads or borrow friction.
- **Neighborhood lifecycle cleanup:** `NeighborhoodStore::delete_neighborhood`
  and `import_directory` purge autosave rows for that neighborhood ID inside
  the same atomic write transaction so deleted or re-imported neighborhoods
  never leave orphaned saves behind.

## What Would Reopen This

- If `Saved` (`sims-vm::boot::save::FORMAT`) gains an analogous cross-version
  migration chain once PR #340 lands, `sims-vm` can reuse `MigrationRegistry`
  and `decode_postcard_exact` directly without changing `sims-sim::migration`.
- If Track 1's `sims-game` background save worker needs bounded retention over
  unrecognized/corrupt slots or async cancellation metadata, `AutosavePolicy`
  and `SaveJob` can be extended in a backward-compatible `AUTOSAVE_FORMAT = 2`
  migration step.
