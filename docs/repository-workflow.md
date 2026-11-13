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

### Active package-opening restore cuts

The same ignored Open diagnostic can request exactly one observed active cut
with `SIMS_MAGIC_PACKAGE_OPENING_RESTORE_CUT=animation`, `creation`, or
`placement`. Use a fresh `SIMS_MAGIC_PACKAGE_OUTPUT` for each attempt. The
headless driver observes real synchronous-service boundaries; it does not
inject an animation, object, placement request, or replacement action.

A cut requires the selected resident's exact active Open serial and the
unopened delivered package. Animation requires its actual package-origin
request and retained animator yield. Creation requires a package-content
factory continuation; placement requires a package-content Find Location
continuation. Missing a requested boundary fails the run rather than earning
restore coverage. The private receipt retains the request, cut tick, original
content identities, dialog progress, and exact checkpoint digest.

The probe writes `during-opening.private.json`, restores those persisted bytes,
and requires exact simulation equality before continuing. This is explicitly
same-process serialized-restore evidence, not separate-process or native
qualification. The original gameplay phase deadline remains unchanged; a run
that captures and restores a cut but later exceeds its deadline is still failed.

A separate test process can continue a recorded active cut:

```sh
unset SIMS_MAGIC_PACKAGE_RESUME SIMS_MAGIC_PACKAGE_OPENING_RESTORE_CUT
export SIMS_MAGIC_PACKAGE_OPENING_RESUME=/absolute/path/to/ignored/cache/prior-cut-run
export SIMS_MAGIC_PACKAGE_OUTPUT=/absolute/path/to/ignored/cache/fresh-active-resume
```

Run the same exact ignored Open test selector above, retaining the same asset,
household, actor, and routing settings. Active-cut resume requires the recorded
natural-delivery proof, a valid named cut, complete consistent continuation
metadata, an exact checkpoint digest, and a recorded capture process distinct
from the current test process. Before any service or player input,
the separate-process restore must reproduce the entire checkpoint byte-for-byte.
The recorded Open action must still be active; it is not submitted again.
Restoration is startup, just as for the before-Open resume; the unchanged
opening-phase limits apply to the resumed suffix. Its usual full contents,
placement, containment, action-return, and repeat-Open checks must still pass.

A prior cut receipt need not claim completed Open: a bounded run may have
captured an exact cut before a later failure. Such a failure is never erased
or turned into a passing original run by a successful continuation. Compare
the resumed final simulation against an uninterrupted control from the same
build and starting input, and report presentation differences separately.
Every checkpoint and raw report remains private under the output rules above.

## Native window observations

`sims-room --original <house> --window-session-output <new-cache-directory>`
records production native-window input receipts, immutable application
observations, root widget draws, scene preparation/encoding, and selected
worker/publication wall spans. The destination must be a new directory beneath
the checkout's ignored `cache` tree; use an isolated `--save` destination there
as well. The option cannot be combined with exploration or screenshot drivers.

Recording is opt-in and does not supply gameplay services or action results.
`--window-session-present-library <library>` additionally enables the optional
macOS presentation adapter. Widget draws are not displayed frames, wall spans
are not CPU time, and input-to-effect draws are not input-to-photon latency.
Retain complete raw records and check dropped-record/session-end diagnostics.

The observation capability object explicitly marks unavailable action/route
terminal events and transport-readiness data as unsupported. Their corresponding
fields are null, not successful completion or readiness. Dialog observations
retain offered response identities without copying original dialog prose;
successful responses still go through the production VM validation path.
Native telemetry alone does not establish long-session or gameplay acceptance.

## Sprite decode reuse

Object asset loading reuses successful decoded SPR2 frames within one immutable
document and byte-identical document reads owned by the same original-house
renderer. Standalone asset loads start with an empty memo.
Each DGRP layer retains its own texture-vector entry
and index; repeated frames share immutable RGBA/depth samples. Document reads,
sprite lookup and DGRP validation retain their original order. Empty frames
still precede depth validation, and failures are not cached. Full document bytes,
including palette data, must match before samples from a previous read can be
reused. Any byte change resets that source's frame store, and a failed changed
document cannot certify partial frames against its old bytes. Complete document
bytes move into the memo only after successful loading. The memo belongs to the
renderer and resets with its existing resource replacement lifecycle; it is not
a disk cache or a replacement for document reads/validation.

Run the synthetic equivalence and isolation tests with:

```sh
cargo test --locked -p sims-render --lib decoded_sprites
```

Installed-asset performance or gameplay acceptance requires separate exact-build
evidence; these tests alone establish neither native responsiveness nor parity.

The renderer also retains one deterministic grass texture across resource-growth
rebuilds. Every bind first validates the current map's presence, shape, stride
and texture-size bound. All map bytes and metadata must match for immutable
pixel reuse; each bind still publishes the current texture index and UV size.
Renderer replacement resets this derived cache. Surface reads, pool binding
and their error ordering remain unchanged. Standalone floor binding retains its
uncached path.

```sh
cargo test --locked -p sims-render --lib grass::cache_tests
```

The ignored `original_house::ground_reuse_tests` differential uses installed
inputs selected by `SIMS_RENDER_TEST_ASSET_ROOT`, `SIMS_RENDER_TEST_BEFORE_SAVE`
and `SIMS_RENDER_TEST_AFTER_SAVE`. It compares complete Assets and Scene values
against a cold-ground renderer, retained image identity and unchanged session
captures. It does not run gameplay or establish native-window acceptance.

## Retained saved-array sources

`sims_content::house_startup::saved_arrays::File` retains an already opened
native `Iff` and its actual stream without replaying Open or constructing an
empty source. `File::from_returned` requires the supplied source operation to
have actually returned; resume a cut by retaining/deserializing the entire file,
not by reconstructing it with that constructor. `Source` borrows that file and
the caller's saved-world services. Array
lookups, resident payloads, callback flags, native error words and releases use
that same IFF owner; allocation, legacy temporary construction/destruction and
array naming use the supplied services. No source file, incoming matrix or
service owner is reconstructed. Unsupported callbacks and out-of-width lease
identities stop before touching the source. A host error inside an IFF operation
retains a fail-closed fault with the file: retry/restore cannot reinterpret a
partially allocated resident payload as a completed lookup. Resuming that
unreturned source operation needs its original continuation, which this
adapter does not implement. Returned native error statuses do not set that fault.
The serialized fault field is required even when null; omission and unknown
file-owner fields are rejected rather than defaulted into a ready source.

Keep the matrix/world read cursor, all destination allocations, converter
scratch, IFF state, stream and service state together across a cut. The borrowed
adapter is not itself a checkpoint and does not complete the saved-world,
surface-map, Gtab/objt or incoming-world bootstrap pipeline. In particular it
does not qualify the historical `Incoming::prepare_read` suffix or initialize
the missing native active-map owner.

```sh
cargo test --locked -p sims-content --lib house_startup::saved_arrays
SIMS_SAVED_ARRAY_TEST_HOUSE=/absolute/path/to/House.iff \
  cargo test --locked -p sims-content --lib \
  house_startup::saved_arrays::tests::installed_house_array_sources_match_direct_native_iff \
  -- --ignored --exact
```

The ignored installed-input test compares direct native IFF queries with the
adapter, including complete resident payloads, missing/read statuses and all
IFF state after query/release. It verifies unchanged source-file bytes, not
gameplay, fresh constructor provenance or native-window acceptance.

## Surface name lookup

Floor and wall material loaders build one borrowed filename index per operation
instead of repeatedly splitting every indexed basename. Each lookup still reads
and compares every matching provider through the ordinary selector, including
its original ambiguity and first-error behavior. No decoded material, source
bytes or missing-file decision is cached by this index.

```sh
cargo test --locked -p sims-content --lib surfaces::resource_names_tests
```

## Definition scan buffers

Object-definition discovery reuses one scan-local byte buffer. Every resource
still receives a complete current read with the ordinary storage, pathname,
bounds and seek checks, followed by whole-IFF validation. No file bytes,
definitions, absence decisions or errors are cached. The allocation ends with
the query. Observed startup-input reads retain their existing owner path.

```sh
cargo test --locked -p sims-content --lib resource_scan
```

The ignored `definition_buffer_tests` differential mounts each checkpoint's
owned schema1 neighborhood over installed assets, compares complete definitions,
read order, errors and Program values against the former allocating parser,
and verifies unchanged checkpoint values. It requires
`SIMS_DEFINITION_TEST_ASSET_ROOT`, `SIMS_DEFINITION_TEST_BEFORE_SAVE` and
`SIMS_DEFINITION_TEST_AFTER_SAVE`. Ordered timings are diagnostics, not native
responsiveness evidence.

## Animation catalog parsing

The live asset host retains a bounded, in-memory memo of successful BCF parses.
Managed restore seeds it while performing its existing animation linking; it is
not serialized gameplay state. Every call still reads every catalog in index
order and validates its complete current bytes and parser mode before reuse.
Errors, aliases and missing-provider decisions are not cached. CFP reads, source
hash reads, conflict checks, reports and partial Program mutation keep their
original paths. A changed input invalidates its old parse before decoding.
Independent hosts own their memos; trusted forks share only immutable payloads.
The 64 MiB input-byte / 16,384-entry limits fall back to ordinary parsing.

```sh
cargo test --locked -p sims-content --lib assets::named_animation_tests
cargo test --locked -p sims-content --lib assets::animation_catalogs
```

Cold and warm differentials use the frozen loader reference, including ordered
read failures and parser modes. These checks do not qualify native latency.

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
