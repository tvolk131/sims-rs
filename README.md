# sims-rs

An unofficial, experimental Rust compatibility engine and set of inspection tools
for the original **The Sims** game formats and behavior. This is a work in
progress, not a replacement game distribution or a claim of complete gameplay,
rendering or save compatibility.

**No original game, executable, assets, dialogue, fonts, screenshots or player
saves are included.** Any game content needed to exercise the engine must be
obtained separately, used with the necessary rights, and kept private.

## Project layout

| Crate | Responsibility |
| --- | --- |
| `sims-formats` | Archive, resource and binary-format readers. |
| `sims-content` | Installation/resource lookup, content loading and save adapters. |
| `sims-vm` | Script VM, continuation state and bounded compatibility services. |
| `sims-sim` | World, object, routing, animation and simulation behavior. |
| `sims-runtime` | Runtime/session orchestration and comparison tools. |
| `sims-render` | Software/GPU rendering and visual state. |
| `sims-app` | Experimental room viewer and archive inspector. |
| `sims-tools` | Command-line inspection and development tools. |
| `sims-fixtures` | Content-addressed local evidence storage and integrity checks. |

The implementation contains explicit partial/unsupported paths. A decoded file,
a passing component test or a saved JSON snapshot does not establish complete
original-engine parity or an original-game-readable save. Keep backups and do
not test against the only copy of a player's installation or saves.

## Build and public tests

The current CI toolchain is Rust 1.99.0. Start with:

```sh
cargo check --locked --workspace --lib --bins --tests
cargo test --locked --lib -p sims-formats -p sims-vm -p sims-sim \
  -p sims-content -p sims-runtime -p sims-fixtures -- --test-threads=1
python3 -B -m unittest discover -s scripts -p 'test_*.py'
```

GPU/application builds also require the native window-system dependencies for
your platform. Linux builds explicitly enable Iced's Wayland backend without
enabling its unrelated default features; running the GUI requires a Wayland
session. On Ubuntu, CI installs `libwayland-dev`, `libxkbcommon-dev` and
`libegl1-mesa-dev`. Runtime tests that need a GPU, game content or a private
observation are separate from the asset-free library unit suites above.
Manual research examples are not part of that CI check; known pre-existing
example compile issues are recorded in the repository workflow.

See the [test selector notes](docs/repository-workflow.md#avatar-codec-test-selectors)
for consolidated integration targets and the
[expansion journey boundaries](docs/playable-alpha.md#expansion-journey-boundaries)
for the distinction between implemented components and complete gameplay.

Explore the CLI without opening a game or a GUI:

```sh
cargo run --locked -p sims-tools --bin sims -- --help
cargo run --locked -p sims-app --bin sims-room -- --help
cargo run --locked -p sims-fixtures -- --help
```

### Private evidence is opt-in

Captured reference corpora and tests containing original-game bytes or prose
are deliberately absent from the public repository. Their dependent tests are
gated behind `private-fixtures`; some otherwise synthetic tests share those
private test modules. Do not use `--all-features` as a self-contained public-clone
check. The feature does not download evidence or grant redistribution rights.

A maintainer with the separately retained local inputs can deliberately run, for
example, `cargo test --locked -p sims-vm --features private-fixtures --lib`.
A public clone cannot reproduce every private comparison, and no extra parity
coverage is claimed merely by omitting those comparisons. New public test data
should be genuinely synthetic, with its provenance explained.

## Native-window automation (opt-in, macOS driver)

The application has an opt-in production-window recorder. It does not supply
gameplay commands or change simulation state. The companion driver posts native
keyboard events to the selected application PID and routes native WindowServer
pointer events to its window, verifying focus, geometry and accessibility hit
testing before each pointer event. It waits on recorded
application-state predicates, rather than invoking engine handlers directly.
macOS 14 or newer is required by the ScreenCaptureKit window-capture helper.
macOS must grant the requesting app Accessibility/event-posting and Screen
Recording permission. A failed permission check stops the run; there is no
headless or direct-handler fallback.

Use an optimized application build for interactive play and responsiveness
qualification. An unoptimized debug build can fall behind the native elapsed
clock and accumulate work before a control command can safely apply. Do not
compare different build profiles as if they were a measured code-only speedup.

Build the application and helper, then run a private scenario with:

```sh
cargo build --release --locked -p sims-app --bin sims-room
swiftc scripts/window_session_macos.swift -o target/window-session-macos
target/window-session-macos permissions --request
python3 -B scripts/window_session.py --binary target/release/sims-room \
  --native-driver target/window-session-macos --root /path/to/private/install \
  --load cache/private-player-save.json --plan cache/private-window-plan.json \
  --output cache/new-window-run --timeout 180
```

Plans use `{"schema":1,"steps":[...]}`. Each named step contains exactly one of
`wait` (a list of `path`/`op`/`value` predicates), `remember` (an observation name),
`input` (`kind: key` with a macOS key `code`, or `kind: click/move` with a logical
content `point: [x,y]`), or `capture` (an alphanumeric filename stem).
An `observe` operation continuously checks a nonempty `conditions` predicate
list for `duration_seconds` (1–300). Optional integer `min_tick_progress` requires
actual simulation progress; `min_frames` defaults to at least one fresh draw per
second, with a minimum of two. A fresh duration-end draw is required, with no
more than one second of closing grace and no extension of the session deadline.
The first eligible duration-end draw fixes the closing batch's final draw ID.
Every draw through that fixed endpoint is checked; later arrivals cannot move
the endpoint indefinitely. Global error/loss checks still cover all ingested records.
For example:

```json
{"name":"playing-soak","observe":{"duration_seconds":30,"conditions":[{"path":"playing","op":"eq","value":true}],"min_tick_progress":100,"min_frames":30}}
```

Key receipt mappings currently cover Enter (36), Space (49), Escape (53), and
the arrow keys (123–126). Input steps require matching, ordered application-side
Iced key/pointer receipts; an unrelated redraw cannot acknowledge input.
Explicit state predicates are still needed to prove a gameplay effect.
The native helper defaults to an 80-millisecond mouse press and a
20-millisecond key press. A click or key action can specify integer `press_ms`
between 1 and 1000; move actions cannot. The requested duration is recorded in
the native helper output. This controls synthetic press duration, not human
reaction time or an input-to-photon measurement.
Pointer events use the macOS session event tap only after foreground, window
geometry and accessibility hit-test checks. Keyboard events target the launched
process. Failure cleanup sends releases through the same route as their presses;
a cleanup attempt never turns a failed input into a passed step.
Predicate operators are `eq`, `ne`,
`gt`, `ge`, `lt`, `contains` and `not_contains`; `ref` can reference a remembered observation
instead of `value`. `any_match` applies its `value` list of 1–64 simple predicates
to one record in an array; all must match that same record. Nested collection
predicates are rejected. Use this for retained terminal events when several
actions retire between visible draws, rather than assuming the latest event
belongs to the action under test. For example a readiness predicate is
`{"path":"tick","op":"ge","value":0}`. Use an existing supported player-save
envelope; the driver does not repair saves or invent missing gameplay state.
Wait lists must be nonempty, and step names, observation names and capture stems
must be unique. Unknown input operations are rejected.
Waits consume fresh observations chronologically, including transient states
after native input. An immediately following `remember` pins the exact matched
draw, rather than taking a newer snapshot. Input fences reject stale pre-input
evidence. Every newly consumed draw must satisfy a continuous observation;
elapsed wall time alone never establishes simulation progress.
Live ingestion validates every record but retains immutable JSON bytes instead
of millions of decoded object graphs. Frames and absolute-indexed input receipts
decode on demand; full report materialization follows application exit. This
keeps growing telemetry history out of live cyclic-garbage-collection scans
without disabling collection, omitting evidence, or changing predicates.

Outputs are restricted to a new directory beneath this checkout's ignored
`cache/`; a symlinked cache root is rejected. They include raw input/dispatch
observations, immutable application/shell observation IDs and scene-content
identities, UI-draw IDs, scene CPU preparation/command-encoding times, draw intervals,
and local window screenshots. Loading frames are excluded from the ready-window
timing distributions. Recording overhead and capture steps still affect timing;
these are diagnostic observations, not a controlled throughput benchmark.
Opt-in records expose worker queue age and phase spans for execution and
publication services.
Their per-phase `wall_us` distributions include loading and measure elapsed wall
time, not CPU or GPU execution; overlapping spans are not additive. Malformed,
unpaired or incomplete spans invalidate the evidence. Passive action-retirement
and native-route outcome witnesses are bounded, private diagnostics, disabled
in normal play and excluded from saves and preview clones. Lost observations
invalidate the run; queue disappearance or a previous-action record alone never
proves normal completion.
Nonvisual elapsed-clock diagnostics are recorded separately under
`state.diagnostics.clock`; wall-clock polling while paused does not change the
application/shell/scene identity used for screenshot attribution.

Screenshots require a paused, settled state and record the surrounding draw-ID
range. An observation-only fence file requires a new draw after capture finishes;
queued pre-capture observations cannot close that range. These fence tokens never
enter the simulation or supply gameplay commands. The run fails if application,
shell, scene, input sequence or cursor identity changes across capture. This is **not
an exact compositor-frame or presentation timestamp**. Without the optional
observer, completed presentation is unavailable. GPU duration, displayed FPS
and input-to-photon latency remain explicitly unavailable,
not inferred from CPU draw timings. CPU widget timing covers the observed root
widget, not separately rendered overlays or the complete Iced/compositor pipeline.
Missing scene-stage telemetry, any observed application error and
unconfirmed graceful shutdown fail the run. Binary, helper, runner, plan and input hashes
are checked before and after execution; changing them invalidates the run.
Game data, plans, logs and captures must remain private and must not be attached
to issues, CI artifacts or releases.
See the [bounded session scope and evidence limits](docs/native-window-session-20261103.md).

### Experimental presentation diagnostics

An opt-in, authored macOS observer supports bounded local presentation
measurements. The supported-household release preview passes the three
consecutive extended cohorts documented in the
[performance preview report](docs/performance-preview.md), including its
explicit remaining background-engine limits. This qualifies that recorded
journey on its recorded display, not universal performance or platform coverage.
Ordinary play does not load this library or replace any platform methods.

```sh
sh scripts/build-window-present-macos.sh
```

For a diagnostic run, add `--present-library target/window-present-macos.dylib`
to the native-window runner command. The application loads it explicitly; no
inherited injection environment variable or dependency patch is required.
The observer records Metal drawable acquisition, exact draw/state/texture
bindings and `MTLDrawable.presentedTime` callbacks. The validator reconciles
individual acquisitions, registrations, bindings and callbacks; checks declared
host-clock metadata and lifecycle boundaries; and classifies foreground status
at presentation time, not callback delivery time. Bracketed Mach absolute and
continuous clock samples cover measurement and drain; an offset change beyond
sampling uncertainty, or a sampling bracket wider than one millisecond,
invalidates the run. No in-process `NSWorkspace` initialization is required.
Startup display-change intervals are explicitly disclosed and segmented;
display changes after ready presentation begins invalidate the evidence.
The native helper records the target window's current display-mode dimensions,
reported nominal refresh and screen maximum. Zero nominal refresh means unknown
or adaptive, not zero Hz; these fields do not establish instantaneous refresh.
Instrumented runs require consistent, active display snapshots. Zero-timestamp
callbacks remain explicitly counted as unmeasured outcomes, never invented
on-screen presentations or proven display drops. Ready outcomes still require
their own exact bindings and complete acquisition/callback accounting. Timing
uses known positive-to-positive presentation gaps, retaining the entire gap
across intervening zero outcomes. Ready pacing is measured between adjacent
ready positives even if loading positives intervene. Zeros cannot extend
measured duration or shorten censored leading/terminal gaps; missing callbacks
still invalidate the evidence.
Texture reuse before callback delivery is counted and joined to its original
acquisition, not confused with a new drawable's binding.
Every ready presentation requires its own exact scene-texture binding; it cannot
borrow another acquisition's binding merely by sharing a draw identity.
Post-cutoff presentations
remain accounted for but do not enter measured intervals because platform
observation stops at cutoff. Missing, unresolved or inconsistent evidence
invalidates the distributions. These timestamps do not measure physical pixel
response, scanout completion, GPU execution or input-to-photon latency.

Short diagnostic success does not establish sustained qualification.
Observer-fence files are sampled on a recorder-only background thread. Root
draws take an atomic snapshot and publish that token as their drawn fence;
input receipts still use the most recently drawn token, not an unseen poll.
A failed/incomplete read retains the last complete token and cannot acknowledge
a later fence. Root-stage diagnostics report disjoint wall intervals, including
the fence snapshot, outside immutable gameplay state; they are not CPU timings.
`qualify_presentations` separately requires at least 900 seconds between ready
presentations, p95 intervals at most 20 ms, p99 at most 33.4 ms, and separately
bounded leading/terminal presentation gaps of at most 250 ms. The maximum
complete ready presentation interval must also be at most 250 ms, so sparse
internal stalls cannot hide behind percentiles. Post-cutoff
callbacks cannot hide an untimed terminal stall or extend the measured duration.

`scripts/window_controls.py RUN --endpoints cache/private-endpoints.json` measures
response to explicit, plan-hash-pinned effect predicates. The schema-1 endpoint
file contains `plan_sha256` and an ordered `controls` list of `step`/`conditions`
entries, one per native input. The analyzer checks the actual remembered draws,
fresh exact receipts, closing acknowledgments and effects before the next
command. Already-true effects are rejected. It reports application-side receipt
to effect-draw latency, not action completion or input-to-photon latency; the
separate control target is p95 at most 100 ms and maximum at most 250 ms.

### Local preview launcher

`python3 -B scripts/play_preview.py` launches a locally prepared, qualified
bundle described by the private `cache/performance-preview/manifest.json`.
**No qualified bundle is supplied by the public repository.** The launcher
refuses a missing/unqualified manifest or changed executable/save. A `passed`
flag alone is insufficient: before opening the window it verifies hash-bound
result/raw-telemetry evidence for three distinct consecutive native attempts,
checks their common build/input identities, and recomputes the control,
display and extended-presentation gates using the checked-out analyzers.
Plans and endpoint definitions are consumed from hash-verified in-memory
snapshots. Each raw trace is streamed into a private, hash-verified read-only
snapshot shared by the control and presentation analyzers, then removed;
every retained verification input is checked again before acceptance.
Complete process start/exit envelopes and validated presentation lifecycles
must be nonoverlapping, not merely the native helper command intervals.
This scans the retained private traces; it is local evidence verification,
not cryptographic build provenance or protection against malicious checkout or
game-data changes. Each launch
creates a separate private session, copies and verifies a read-only executable
and supported starting save, starts paused and directs UI saves to a distinct
new path. It executes that private binary snapshot rather than the mutable
bundle path, so rebuilding the bundle cannot replace the launched program.
It prints that session's output location and records the launch identity there;
it never downloads game data or overwrites the supported input.
The [local handoff and measurement report](docs/performance-preview.md) describes
the qualified run results, native Save/reload smoke, usage, and known limits.

## Licensing and provenance

Project-contributed source and documentation use the [MIT license](LICENSE).
Dependencies and consulted upstream projects have separate terms; see
[third-party notices](THIRD_PARTY_NOTICES.md) and the
[dependency license inventory](docs/dependency-licenses.md).

Read [licensing and publication boundaries](docs/legal-and-provenance.md) before
contributing or distributing builds. Development used original-engine analysis
and open-source format references: **this is not a clean-room claim**. The MIT
license does not grant rights to Electronic Arts/Maxis content, upstream code or
trademarks, and an engineering audit is not legal clearance.

The initial public commit is a new, source-only history. Earlier local history,
research notes, capture scripts, native fixtures and private backups are not
published. See the [repository workflow](docs/repository-workflow.md) for upload
gates and contribution requirements. Please do not attach original-game data,
captures or screenshots to issues or pull requests.
