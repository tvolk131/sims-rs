# sims-fixtures

Local evidence storage for simulation and rendering tests. This crate depends
on neither the VM nor the renderer. It stores data and verifies integrity;
scenario execution and semantic comparisons belong to their respective runners.
It does not change the format or location of player saves.

See the [publication boundary](../../docs/legal-and-provenance.md) before retaining
or sharing captures. This crate stores private evidence; publishing its source
does not authorize redistribution of the evidence it can hold.

## What is retained

A small, versioned JSON case manifest describes:

- Exact original source bytes, independently of normalized observations.
- Ordered inputs, waits for completed ticks, and named non-tick boundaries.
- Observed reference states, or resumable states with an explicit restore contract.
- Expected observations, stored as bounded binary trace chunks with keyframes,
  object deltas and chained hashes. Arrays are replaced atomically in this version.
- Screenshots, dimensions/scale, and camera/UI configuration artifacts.
- Source hashes, inert command arguments, provenance, and explicit coverage limits.

An empty imported schedule does **not** establish that no input occurred.
Imported observations do **not** establish a restorable VM, executed scenario,
or strict original-engine parity. A source-provided hash remains a declaration
unless a separate verification report is attached. Missing screenshot camera/UI
settings remain explicit `null` rather than invented defaults.

Runner adapters should write compact `RunReport`s for successes. On a failure,
attach the first divergent state, relevant trace segment and optional screenshot
as report artifacts; use `Pin` to retain investigation-only evidence. The library
does not automatically record every tick or take screenshots.

## Store format v1

```text
store.json
cases/<case-id>/manifest.json
blobs/sha256/<first-two-hex>/<remaining-hex>.blob
runs/<run-id>.json
pins/<pin-id>.json
tmp/
```

`BlobRef` contains a SHA256 of the **uncompressed bytes**, plus their exact length.
Identical bytes deduplicate regardless of the requested storage codec. New blobs
are written to temporary files, flushed, and published without replacing existing
content. Existing deduplication hits are verified before being accepted.
Case IDs are immutable: identical publication is idempotent; changed content
requires a new ID. Interrupted imports may leave unreferenced completed blobs,
but never publish an incomplete case.

Each blob has a 49-byte header: `SIMSBL01`, one codec byte (`0` raw, `1` gzip),
decoded length as little-endian u64, then 32 digest bytes. Typed payloads use
named MessagePack with explicit schemas. Gzip uses the pure-Rust `flate2`
backend. Typed Rust float bits, including NaN payloads and signed zero, survive
roundtrips; original JSON text is also retained exactly. JSON observations cannot
represent NaNs that their source never contained.

Compression is a storage concern, independent of content identity. A different
codec can be added with a new codec tag; no database server or custom page engine
is required. The case manifests and pins supply the initial index. A database
index can be added later if measured lookup workloads justify it.

Artifact references declare their complete dependency closure. Recognized
`TraceIndex` and `ArchiveInventory` payloads are decoded and their declared closure is checked;
opaque application payloads require accurate producer declarations. Verification
rejects unsupported schemas, missing/corrupt blobs, length/hash mismatches,
trailing compressed data, changed trace order and invalid deltas.

This is a trusted local store. Symlinks are rejected when inspected; it is not a
sandbox against a process concurrently replacing filesystem paths. Streaming
decode/reconstruction can deliver a verified prefix before discovering a later
error. Consumers needing all-or-nothing output must stage their result and
publish only after the whole operation succeeds.

Exports copy and verify a case's declared closure into a new independent store.
They never hardlink to the source store. Existing destination directories are
rejected. The store marker is published last; an incomplete export cannot be
opened as a valid store.

## CLI

Use a private corpus outside disposable Cargo output, for example
`fixtures/private/corpus` (already ignored by Git). Commands require an explicit
store path and never scan the old cache automatically.

### Direct capture without a raw spool

`capture --id NEW-ID` streams exact stdin bytes directly into a gzip blob and
publishes an unqualified source case at EOF. It validates the case and opens the
temporary compressed sink before emitting a one-line readiness acknowledgement
on stdout. The remaining stdout is a final JSON report with the source BlobRef.
The default decoded-byte cap is 16 GiB; `--max-bytes` accepts 1 byte through
64 GiB. Overflow rejects publication and cleans the temporary blob. Existing
case IDs are rejected before readiness. EOF does not prove producer success,
valid JSON, a completed tick, resumability or an executable input schedule.

`source CASE-ID --sha256 HASH --bytes N` streams that case's source only if its
verified identity matches both arguments. The consumer owns payload decoding.
It checks the complete case before emitting bytes and validates again while
copying; consumers still need to check the process exit status.

The native tick and load-phase capture scripts have an explicit fixture-output
mode. Add these options to an otherwise validated capture command:

```text
--fixture-store fixtures/private/corpus
--fixture-id new-capture-v1
--output cache/new-capture.fixture.json
```

Build the release CLI first. `--fixture-cli` selects another explicitly supplied
CLI path. Fixture mode requires the distinct `.fixture.json` extension; it never
puts a reference stub at a raw `.jsonl` path. The small receipt records the
source identity, relative store location, producer invocation and whether the
producer returned. Exceptions retain the exact emitted prefix and mark the
producer unsuccessful. A killed producer may leave a published unqualified case
without a receipt; case publication occurs at stream EOF, not per row. Include
`fixture_output.py` in any newly frozen Python capture source bundle.

`compare_household_ticks.py` and its process-page verification accept these
receipts as inputs, preserving decoded hashes, lines and comparison rules.
Unsuccessful producers, corrupt/missing content and mismatched source identities
fail closed. Receipt/provenance command strings are never executed. Existing raw
capture and comparison commands retain their format and behavior. This is an
opt-in capture/reader integration; Rust scenario runners and other legacy dump
writers still need deliberate adoption.

```sh
cargo run --locked -p sims-fixtures -- --store fixtures/private/corpus init
cargo run --locked -p sims-fixtures -- --store fixtures/private/corpus \
  import-jsonl /path/to/selected-capture.jsonl --id example-v1 \
  --record-limit 33 --tick-pointer /tick \
  --note 'Selected observations; parity and restore remain unqualified'
cargo run --locked -p sims-fixtures -- --store fixtures/private/corpus verify
cargo run --locked -p sims-fixtures -- --store fixtures/private/corpus inspect example-v1
cargo run --locked -p sims-fixtures -- --store fixtures/private/corpus \
  export example-v1 /path/to/new-independent-export
cargo run --locked -p sims-fixtures -- --store fixtures/private/corpus gc --dry-run
```

`import-json` retains an entire bounded JSON input. `import-jsonl` reads a bounded
prefix (33 records by default), retaining CRLF/LF and a missing final newline
exactly. It does not skip blank or invalid lines. Byte, record and normalized
trace caps are explicit CLI options; the JSONL import defaults permit legacy
large records but remain bounded. `--tick-pointer` records an observed unsigned
tick, without inferring completed ticks from line numbers. `--advanced-pointer`
accepts Boolean source fields only; omit it for native numeric `0`/`1` flags,
which remain intact in the source and observations. `get --sha256 HASH --bytes N`
copies decoded bytes to stdout or a new `--output` file.

GC is **preview only**. There is no deletion API or destructive CLI flag. Old
captures, frozen binaries, VM disks and saves are never removed by this crate.
Migration should select reusable references, retain source-span receipts,
verify exact bytes/reconstruction/export, then separately decide retention.

`examples/legacy_pilot.rs` performs this storage proof for a selected original
Newbie capture plus a source-span receipt: exact byte fingerprints, every
normalized observation, observed start/end references, independent export and
reconstruction. It requires fresh destination paths and reports measured sizes
and timings. It earns zero additional gameplay parity ticks.

```sh
cargo test --locked -p sims-fixtures
cargo clippy --locked -p sims-fixtures --all-targets -- -D warnings
```

Tests cover named binary/byte/float roundtrips, bounded reconstruction,
corruption, failed writers, immutable publication, dependency omissions,
read-only GC, independent/concurrent exports and legacy import behavior.

## Lossless archival of existing files

Reference selection requires judgment: a legacy capture can contain useful
failure or source evidence even when no new-store case references it yet.
Store reachability alone is not authorization to discard that evidence.
Archive inactive full files before retiring their raw copies; keep current
inputs, source bindings and cheap diagnostic journals readily available.

```sh
# paths.json is an explicit array of paths relative to cache, not a glob.
cargo run --locked -p sims-fixtures -- --store fixtures/private/legacy-archives init
cargo run --locked -p sims-fixtures -- --store fixtures/private/legacy-archives \
  archive --source-root cache --files-from paths.json --id old-states-v1
cargo run --locked -p sims-fixtures -- --store fixtures/private/legacy-archives \
  restore old-states-v1 /tmp/restored-old-states
cargo run --locked -p sims-fixtures -- verify-restored /tmp/restored-old-states
```

Each versioned archive inventory retains original paths, complete raw byte
hashes/lengths, modified time, and available original Unix mode/device/inode/link
metadata. All file blobs are declared in its independently verified dependency
closure. Hardlinked source names remain distinct paths with deduplicated blob
content. Source mutation aborts publication; path conflicts, symlinks, missing
data and changed hashes fail verification.

Restore creates a **new** bundle containing `files/<original-relative-path>`,
`inventory.msgpack` and a completion marker. It verifies staged bytes before
publication and never overwrites an existing destination. A failed publication
may leave an incomplete directory without a valid marker; it is not a successful
restore. `verify-restored` needs neither the original source nor the store.
Restored files have private/default permissions; timestamps, privileged bits,
ownership and original hardlink topology are not recreated. Their original
metadata remains available for audit.

To use an archived checkpoint with a historical script, first restore the case,
then copy the required file from the bundle's `files/` tree to its missing
original path or pass the restored file explicitly. Current active consumers
should be migrated deliberately before their raw inputs are retired. Archive
and restore commands never delete source files. Any separate retirement must
verify a full restore, preserve the path mapping and recheck the exact source
before removal.
