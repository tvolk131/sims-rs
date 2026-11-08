# Repository workflow and source-publication gates

## Initial public baseline — November 3, 2026

The public source repository is `tvolk131/sims-rs`; `origin` uses SSH. Publication
uses one new parentless `main` commit, not any commit from the prior research
history. The old local history and pre-sanitization working tree are privately
backed up under `cache/private-publication-backup-20261103/`. Old local references
are retained only in the private Git namespace, not published branches or tags.
Do not upload the backup, push those references, or use `git push --mirror`.

Read [legal-and-provenance.md](legal-and-provenance.md) and
`THIRD_PARTY_NOTICES.md` for the actual scope, audit limits and reference history.
A clean root commit is an upload boundary, not a clean-room claim or a way to
relicense someone else's work.

## Public source versus private evidence

The ignored local installation, cache, saves, extracted data and reference
checkouts remain private. All `crates/*/tests/fixtures/`, `scripts/research/`,
historical research documents and identified source-level private test modules
are omitted from the public tree. These files are not destroyed or converted to
LFS. A `.json`, compressed file or source-code extension does not make their
contents safe to publish.

The `private-fixtures` feature is disabled by default in every workspace package.
It enables explicitly gated local comparisons only when the maintainer separately
has the private inputs. It does not authorize shipping them or make a public clone
self-contained with `--all-features`. Public CI does not restore private evidence,
run the original engine, upload game screenshots or cache captures.

Reviewed JSON records are pinned by path, byte count and SHA-256, with a provenance
statement in `.github/repository-policy.json`. New/changed JSON requires a new
review. The old large-capture exceptions have been removed. Individual blobs have
a 5 MiB limit and the public checkout has a 64 MiB budget. These size limits are
not evidence of copyright clearance.

## Local gates

Activate versioned hooks once per clone:

```sh
git config core.hooksPath .githooks
```

Before staging/pushing:

```sh
python3 -B -m unittest discover -s scripts -p test_check_repository.py
python3 -B scripts/check_repository.py
python3 -B scripts/check_repository.py --staged
python3 -B scripts/check_repository.py --history HEAD
```

The working-tree mode includes Git-eligible untracked files. The pre-commit hook
uses actual index bytes and the staged policy, not unstaged replacements. The
pre-push hook scans all reachable history of each pushed non-deletion tip; its
policy is read from that tip, not an uncommitted policy edit. Historical paths
are checked independently of blob deduplication, so deleting a private file or a
secret in a later commit does not clear its earlier upload.

Checks reject private paths, game/archive/media/build extensions, binary content,
unreviewed JSON, nested Git checkouts, known private payload fingerprints in
common source encodings, long encoded source payloads, basic credential patterns
and unreviewed oversize blobs. Payload values are not printed. The fingerprint
list contains digests and sizes, not the original code or game prose.

These are bounded heuristics. They do not detect arbitrary code copied from an
upstream, every asset encoding, every credential or all legal restrictions. Hooks
can be bypassed and trusted contributors can change policy. Human provenance
review remains necessary; passing checks is not legal clearance.

## CI and release scope

CI runs on pull requests, pushes to `main` and manual dispatch. The check names
are `Repository hygiene` and `Rust checks`. The Rust job pins toolchain 1.99.0,
checks default-feature workspace library, binary and test targets, and runs the
six core libraries' asset-free unit suites serially. It also executes the
synthetic `avatar_codecs` integration target, the asset-free `inventory_operations`
integration target, and the `sims-render` library tests as separate steps.
These renderer tests do not qualify native GPU
presentation or installed-asset loaders. Fixture-gated tests do not silently
count as executed coverage. CI does not enable private fixtures or all features.

Actions are pinned to exact commits. Repository-wide default workflow permissions
are read-only and workflow approval of pull requests is disabled. The initial
setup permits GitHub-owned Actions only. Dependency alerts are enabled. These
settings do not replace code review or a release/license audit.

Use a short-lived branch and a focused pull request after the baseline. Review
new/changed data and any upstream adaptation explicitly. Check the complete
source archive and intended release files, not only the diff. Crates.io publication
is disabled in all workspace packages; distributing binaries needs its own
actual-linked-dependency license/NOTICE inventory and asset review.

Manual, capture-driven research examples are outside the CI target selection.
An attempted all-target check exposed pre-existing initializer incompatibilities:
`completed_host_join` lacks `live::Snapshot::dependent_jobs`, and
`completed_session_join` lacks `View::neighborhood_replacement`. Their
implementations are not changed by this cleanup. `cargo check --all-targets`
therefore remains a known failing broader check; CI success must not be described
as validation of those examples or of every private-feature configuration.

## Avatar-codec test selectors

The synthetic `sims-formats` targets `animation`, `avatar` and `avatar_index` are
grouped in `avatar_codecs`. The three leaf files remain byte-identical in separate
modules, preserving all nine tests. Cargo autodiscovery remains enabled; private
fixtures remain opt-in and are not needed by this group.

Run the full group:

```sh
cargo test --locked -p sims-formats --test avatar_codecs
```

The old target selectors no longer exist. For one former `avatar` case, use:

```sh
cargo test --locked -p sims-formats --test avatar_codecs \
  avatar::skeleton_hierarchy_and_truncations -- --exact
```

Former `animation` and `avatar_index` cases similarly gain `animation::` and
`avatar_index::` prefixes. Require the expected positive test count: an obsolete
`--exact` name can otherwise succeed after running zero tests. Substring filters
containing a module name can select more cases than before.

This removes two integration binaries, not tests or assertions. It does not
change CI's separately executed library suites or establish a general build,
runtime or gameplay improvement.
