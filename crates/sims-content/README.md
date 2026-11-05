# Content loading

## Validated startup reuse

The live character asset-service owner keeps one private, nonserialized startup
catalog memo. It reuses the successful startup result before caller-specific
lazy resolution, not a live avatar or partially constructed character.

Every invocation still validates the complete installation namespace. A
potential hit also performs the recorded archive and resource observations in
their original order and compares fresh bytes exactly, including repeated reads.
Metadata equality or a matching name alone cannot authorize reuse. At the first
changed read or read error, normal startup resumes using the current invocation's
already observed prefix, without rereading it or probing later old dependencies.

Each caller receives independent registry, lookup-attempt, directory, and asset
identity state. Immutable decoded payloads may share allocation until mutation.
Character reconstruction, provenance reads, transaction publication, and RNG
consumption remain outside the memo's reusable result.

### Lifetime and memory policy

- There is at most one entry per live asset-service owner, tied to its exact
  immutable index allocation. Another index conservatively uses ordinary startup.
- The recorded transcript is limited to 32,768 events and 1 GiB of charged
  resource/archive bytes and event overhead. Exceeding either limit abandons
  retention; ordinary loading continues without treating capacity as an error.
- This is **not a 1 GiB total-memory limit**. The namespace, startup template,
  retained index, allocator overhead, validation buffers, and caller-owned results
  use additional memory. It is not a low-memory startup mode.
- Snapshots do not serialize the memo or its descriptor state. Restore starts
  with an empty memo and can populate it through ordinary reconstruction. Forks
  receive a separate empty memo.

This optimization does not skip physical source validation, cache read errors,
or claim that all gameplay hitches are resolved. Synthetic regressions cover
source changes, error precedence, prefix replay, result isolation, retention
limits, construction rollback, restore, and fork behavior.
