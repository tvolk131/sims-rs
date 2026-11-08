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

## Installed-asset diagnostics

The ignored Mystery Man package test drives the headless application's
production input handlers. It observes the original delivery NPC and a newly
created `MagicStarterKit.iff`, then selects that package's real **Open** offer.
It does not inject a replacement package, select the NPC as the player, or
treat an expansion notice as package opening.

Provide your own compatible private installation. No game resources, saves,
captures, original script bodies, or dialog prose are distributed with the
test. Keep the installation and resulting evidence in ignored private
locations, outside a publishable source tree. The output directory must not
already exist. Checkpoints contain retained private resources; failure logs
can contain original dialog text. **Never upload these outputs as CI
artifacts, GitHub attachments, releases, or packages.** Do not point the output
at an original installation or a player-save directory.

Example with explicit household and resident selection:

```sh
export SIMS_PRIVATE_ASSET_ROOT=/absolute/path/to/private/installed-assets
export SIMS_MAGIC_PACKAGE_OUTPUT=/absolute/path/to/ignored/cache/fresh-package-run
export SIMS_MAGIC_PACKAGE_SOURCE=UserData/Houses/House05.iff
export SIMS_MAGIC_PACKAGE_ACTOR=Mortimer
export SIMS_MAGIC_PACKAGE_INFERRED_ROUTING=1

cargo +1.99.0 test --locked -p sims-app --bin sims-room \
  original_viewer::exploration::startup_play::mystery_man::actual_mystery_man_package_open_through_player_input \
  -- --exact --ignored --nocapture --test-threads=1
```

The actor must identify a real resident; runtime IDs are resolved from that
identity rather than hardcoded. The Newbie household has a tutorial
prerequisite. Waiting without its actual inputs does not establish broken
delivery.

Strict routing is the default and can stop at the existing retained native map
prerequisite before delivery. Setting `SIMS_MAGIC_PACKAGE_INFERRED_ROUTING=1`
explicitly selects the existing inferred-routing diagnostic policy, not an
automatic fallback. The receipt labels the policy and always reports
`native_qualification: false`.

The journey allows 6,000 frames and 3,000 completed ticks, 120 seconds before
Open submission and 90 seconds afterward, at most 5,000 service polls per
command, and 32 menu attempts. Phase timers exclude initial loading and cannot
interrupt an engine call. Use an external whole-process timeout with child
cleanup for unattended execution; there is no global instruction cap.
Only genuinely offered responses are submitted. Startup notices and observed
Goth work/school reminders are narrowly pinned; unknown modals stop the run.
The package-owned animation dialog has separate resource and instruction
provenance.

Success requires observed natural delivery (or its explicitly restored
checkpoint), an enabled Open offer submitted through production input, the
package-owned dialog, and the opened attribute transition. It additionally
requires exactly one new table, charger, spellbook, and portal, on-lot
independent placement, bidirectional table/charger containment, successful
completion of the exact queued action, all uncollected-content flags retained,
and no enabled repeat Open or duplicate contents after revisiting the menu.
An opened flag or successful return alone is insufficient because original
creation-failure branches can still reach those outcomes.

`SIMS_MAGIC_PACKAGE_RESUME` may name a previous private output directory with
its receipt and `before-open.private.json`. Resume verifies the recorded
checkpoint digest and matching source, actor, routing, and delivery evidence;
it neither rewrites the receipt nor reports a new delivery. Compare fresh and
restored runs separately. Camera publication counters and a completed camera
command's historical tick can differ after a surface rebuild; do not label
whole checkpoints byte-identical or normalize away those differences.

Open exposed a missing floor-acceptance `ActiveLevel` service. The live host
now reads the existing camera-owned `world.refresh.visible_level` only to
select a rejection diagnostic, not to accept a candidate. Missing ownership
remains an error. Asset-free tests cover view changes across a pending request,
missing ownership, early rejection, and unchanged VM/RNG/work budgets:

```sh
cargo test --locked -p sims-sim --lib lot_acceptance::live::tests:: \
  -- --test-threads=1
```

This is bounded headless gameplay evidence, not strict-routing, native
input/pixel/animation parity, whole-alpha, or travel qualification. It does
not verify every placement coordinate or every intermediate restore cut.
Inventory collection is a separate journey. Add operation zero now retains
the measured signed-quantity behavior: select the caller's requested temp only
when enabled, update the first matching kind/GUID, clamp positive overflow,
preserve signed lower wrap, and append only below the 32,767-entry limit.
Zero quantity succeeds and marks inventory dirty without inserting an entry;
a failed capacity check does neither. Other unsupported operations remain
faults. Asset-free regressions include both owners, duplicates, GUID fallback,
unchanged VM/RNG state, and cold storage boundaries:

```sh
cargo test --locked -p sims-vm --test inventory_operations -- --test-threads=1
```

To attempt collection, keep the same asset/source/actor/routing settings and
provide the exact prior Open output and a fresh private output directory:

```sh
export SIMS_MAGIC_PACKAGE_COLLECT_FROM=/absolute/path/to/ignored/cache/open-run
export SIMS_MAGIC_PACKAGE_COLLECTION_OUTPUT=/absolute/path/to/ignored/cache/fresh-collection-run

cargo +1.99.0 test --locked -p sims-app --bin sims-room \
  original_viewer::exploration::startup_play::mystery_man::collection::actual_package_collection_and_empty_lifecycle_through_player_input \
  -- --exact --ignored --nocapture --test-threads=1
```

The collection test verifies the prior receipt's after-Open checkpoint digest
and natural-delivery provenance. It selects each real menu offer, requires the
exact inventory additions (35 coins, one wand and one of each ingredient), and
keeps the other remaining-content flags intact. Cleared flags or disappearing
offers alone are not transfer evidence. The final interaction is checked as a
removal lifecycle, not assumed to return normally: the effect must be observed
and subsequently disappear, with the package and its live ownership removed.
It retains private checkpoints before collection, before the last item, and
after completion. Its 180-second active phase, 6,000-frame and 3,000-tick bounds
also require an external whole-process timeout. Installed-asset execution is
ignored in ordinary CI; this selector's existence is not a recorded test pass.

For a final-removal continuation check, additionally set
`SIMS_MAGIC_PACKAGE_COLLECTION_RESUME` to a completed collection output,
keeping `SIMS_MAGIC_PACKAGE_COLLECT_FROM` at its original Open output. Use a
new collection-output directory. This verifies the prior run's exact
`before-last-item.private.json` digest and common Open lineage, restores four
already transferred items, and executes only the last real offer and removal
lifecycle. Its receipt explicitly distinguishes that scope from five fresh
transfers. Compare final simulation state separately from presentation rebuild
counters; neither a successful restore nor an observed removal alone proves
equivalent continuation.

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
