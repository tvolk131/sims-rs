# Salvage audit (November 13, 2026)

A read-only audit of every crate, graded against `docs/vision.md`. Its purpose
is to decide what carries into the re-founded workspace. Paths refer to the
`integration/playable-alpha-20261113` branch.

Verdicts:
- **PORT:** carry the code over, with light changes.
- **REFACTOR:** carry it over after restructuring.
- **REFERENCE:** read it for behavior and rewrite it.
- **PARK:** keep it out of the new workspace and revisit later.
- **DROP:** delete it.

## Overall

About 680k lines in total. Roughly **8% is worth carrying as code** (about
50k lines, most of it needing refactoring) and roughly **20% is worth keeping
as reference**. The rest is machinery that mirrors the native executable
(addresses, captured memory, emulated Win32, resumable copies of native call
graphs), plus evidence, receipt and acceptance tooling and tests that depend
on private fixtures.

| Crate | Lines | Carry as code | Reference | Verdict |
| --- | --- | --- | --- | --- |
| sims-formats | 7.3k | ~67% | ~15% | Seed of the new formats crate |
| sims-sim | 187k | ~12–15% | ~25% | Rewrite; port routing, placement and clock kernels |
| sims-vm | 100k | ~10% | ~22% | Rewrite the interpreter; port the pure kernels |
| sims-render | 55k (15k src) | ~10% | ~12% | Port the scene/GPU core; leave the device and dialog layers |
| sims-content | 98k | ~6% | ~30% | New crate; port the index, GUID scan, save import and linker |
| sims-runtime | 129k | ~2% | ~12–15% | Leave behind; port the exploration fuzzer |
| sims-app | 88k | ~2% | ~9% | Leave behind; mine the HUD layout tables |
| sims-kernel | 5.5k | 0 | 0 | DROP (Win32 mutex/event emulation) |
| sims-fixtures | 6.8k | 0 | design only | DROP (evidence store) |
| sims-tools | 0.9k | 0 | small | Rebuild as a thin CLI |

## Cross-cutting findings

**Good news**
- No `HashMap`/`HashSet` anywhere outside `native_bootstrap`; `BTreeMap`
  throughout.
- No `Rc`/`RefCell` webs and no hidden globals in sim code. `Simulation`,
  `Vm`, `World` and `Thread` are `Clone + Serialize`.
- RNG is already sim-owned and exact (`sims-vm/src/rng.rs`, the 0x41c64e6d
  stream with separate lot and CRT streams).
- An exact x87-extended soft-float already exists (`sims-vm/src/numeric.rs`
  `Wide`, with `ftol`). It is the natural core of the sim float wrapper.
- The renderer already supports any resolution and continuous zoom
  (`sims-render/src/lib.rs` `Camera::sprite_zoom`).

**Problems that block the vision**

1. **There is no command layer anywhere.** The UI calls about 35
   `begin_*`/`advance_*` session entry points directly, and render code steps
   the VM (`sims-render/src/dialog/client.rs:410,435`) and writes the image
   cache (`portrait/job.rs:91`). `Session::step()` returns continuations
   (`UiEffect`, `HouseNotification`), which the UI must service before ticks
   can resume (`sims-runtime/src/original_session.rs:1563-1597`).
2. **Presentation state lives in sim state.**
   - `World` holds `camera_rotation`, `image_cache`, `ui_owner`, `refresh`
     draw state, sound and effect queues, and indicator queues.
   - Objects carry `native_flags`, `native_tree_error` and similar fields.
   - The UI drains sim queues (`sims-app/src/original_viewer.rs:1729`).
3. **Scripts read the camera.** The tile-depth primitive
   (`sims-vm/src/person.rs:685`), Generic Sims Call 12, Look Towards and Find
   Location all read `world.camera_rotation`. The app writes it directly
   (`original_viewer.rs:3472`, `laboratory.rs:214`). This makes camera
   rotation a sim input, and it is a new multiplayer policy question (see
   below).
4. **Wall clock enters the sim.** `original_viewer.rs:2052` feeds `Instant`
   time into the emulated native clock, which decides how many lot passes
   run, and inputs land on whatever tick the worker happens to be on.
5. **Text measurement uses system fonts and feeds the sim.**
   `sims-content/src/notifications.rs:57-66,206-214` answers sim `Measure`
   requests with cosmic-text layout over Arial or Liberation Sans. This is a
   drift source between machines.
6. **Float hazards.**
   - std `atan2`/`sin`/`cos`/`powi` in about 10 places (`sims-vm/src/look.rs:166`,
     `sims-sim` locomotion, lot_snap, routing, head_look, pose/movement and
     steering).
   - Two coexisting motive precision modes, one of them an f64 "LegacyExtended"
     stand-in for 80-bit.
   - `f32::round` where x87 rounds half to even (`memory.rs:568`).
   - Double rounding in motive expressions (`memory.rs:827`).
   - About 181 raw `f32` uses in sims-vm alone.
   - TTAB attenuation and FCNS values load without NaN checks.
7. **Content discovery only knows this project's cabinet-extraction layout**
   (`GameData_ALL_Recursive`, `ExpansionPackGOLD`, ...). It supports neither a
   real CD/Complete install nor the Legacy Collection. Custom content
   hard-fails on duplicate names, conflicting GUID providers, non-1252
   filenames and symlinks.
8. **Snapshots are not self-contained.** Restore re-reads the original files
   and rebuilds the program. The format is JSON with a hard version check
   (NaN can't round-trip). There is no REDB code anywhere.
9. **Tests.** About 3,900 tests in total. About 1,000 depend on private
   fixtures that are absent from the checkout, and over 1,000 assert
   native-infrastructure bookkeeping. Roughly **500 public synthetic tests
   encode real behavior or format knowledge** and are worth re-expressing.

## What to carry, by area

### Formats (port first)
`sims-formats`:
- **PORT:** `reader.rs`, `iff.rs`, `far.rs`, `behavior.rs` (BHAV, TTAB
  including compressed v9+), `execution.rs` (GLOB/BCON/OBJf/SLOT),
  `field.rs`, `strings.rs`, `tuning.rs`/`tatt.rs`/`career.rs`, `sprite.rs`,
  `legacy_sprite.rs`, `lot.rs`, `neighborhood.rs`, `saved_objects.rs`,
  `animation.rs`, `avatar.rs`, `avatar/text.rs`, `ffn.rs`, and
  `tests/boundaries.rs`.
- **Changes:**
  - typed errors;
  - a lenient mode that skips bad chunks with diagnostics (stale rsmp,
    trailing bytes);
  - merge `read_bcf`/`read_bcf_native`;
  - move `sprite::render` to render;
  - drop `far/mount.rs` and `pe32` (oracle tooling only).
- **Missing:**
  - full typed OBJD;
  - BHAV 0x8003 tolerance;
  - TPRP/TRCN;
  - the whole audio stack (FWAV, HIT/HSM/EVT/FSC, XA, UTK, WAV);
  - BMP_ chunks and UIGraphics BMP/TGA;
  - DGRP SPR# fallback.

### Content
`sims-content`:
- **PORT/REFACTOR:**
  - `Index` and FAR directory reading (`lib.rs:105-245`);
  - `resource_scan/object_guids.rs`;
  - `saves.rs` `House::load` (import only);
  - the `vm.rs::load_program` linker core;
  - avatar/mesh/texture decode glue;
  - `compatibility.rs` (corpus opcode census);
  - careers and zoning.
- **REFERENCE:**
  - mount and override order (`avatar_installation.rs:56-77`, sims-sim
    `catalog_paths::initial_mounts`);
  - object catalog registration and GUID-collision rules;
  - save-to-world import order (`save_storage.rs`);
  - clothing enumeration.
- **REWRITE:** install discovery, as per-layout binding tables with
  autodetection and one explicit ordered layer list (Downloads last), with
  warnings instead of hard errors.
- **DROP:**
  - `native_bootstrap/` (26.8k);
  - `startup_memo`;
  - `owned_resources`;
  - the `character_*`/`graphic_*`/`autonomy_resources` service layers;
  - the compare/export examples.

### VM
`sims-vm`. Write a new interpreter core of about 3–5k lines:
- dense storage;
- a clean `Status`;
- no observation or native-suspension hooks;
- dialog answers and speed changes arrive as commands.

What to bring from the old crate:
- **PORT nearly verbatim:**
  - `rng.rs`;
  - `numeric.rs`;
  - expression, Random and scope logic from `memory.rs`;
  - curves, decay and scoring from `motives.rs`;
  - the autonomy CRT-qsort tie order and top-N select;
  - `call()` argument semantics (contiguous argument read, `[255;8]` temps).
- **REFERENCE:** `action.rs` (queue rules), `find_best.rs`,
  `relationship.rs`, the `person.rs` primitives, `dialog/`, `idle.rs`, and
  the person factory and reset ordering.
- **Not implemented yet:** opcodes 15, 19 (Make New Character), 21 and 33,
  and most Generic Sims Call subfunctions.
- **DROP:** `registry_suspension`, `native_*`, `retained_*`,
  `scheduler/clock`, `interface`, `ui_effect`, `image_cache`, `presenter`,
  `action_observation`.
- **PARK:** `commander` and the scheduler storage model, until the oracle shows
  they matter.

### Simulation
`sims-sim`:
- **REFACTOR and keep:**
  - routing (`routing.rs`, regions, portals, lot_route, contention, reach,
    locomotion);
  - placement/Find Location;
  - clock;
  - house_valuation;
  - lot_environment;
  - the `saved_*` importer;
  - a pose subset of about 3k lines.
- **REWRITE using as reference:** the scheduler, interaction and removal code
  (about 22k lines, with about 45 stored native continuations).
- **PARK:** lot_cursor, edit_history and Buy/Build (redo as commands in step
  2), and lot_presenter (reimplement projection, cutaway and the depth table
  in render).
- **DROP:** `native_application`/`system`/`game` and
  `native_world*`/matrix/wall (about 35k lines).

### Render
`sims-render`:
- **PORT:**
  - `Scene`/`Camera` (`lib.rs`);
  - `gpu.rs` and `world.wgsl` (per-texel sprite depth);
  - `software.rs` (headless golden-image oracle);
  - `decoded_sprites`;
  - floors, pools and roads;
  - grass;
  - walls and wall objects;
  - `pose::skin`;
  - censor (move it to a shader).
- **Changes:**
  - read the sim through a narrow read-only lot-view trait;
  - real appearance resolution instead of the hard-coded outfit;
  - raise the 4096 viewport cap.
- **PARK/DROP:** `camera_device/`, `dialog/`, portrait jobs, `frozen_house`,
  and the examples. Mine `camera_device/panels` for layout.

### Session and harness
`sims-runtime`:
- **PORT:** `exploration.rs`, the seeded command fuzzer that compares a warm
  fork against a cold restore. It becomes the CI determinism and snapshot
  fuzzer.
- **REFERENCE:**
  - `fixture_replay.rs`: its `Driver` trait, schedule semantics, tick
    monotonicity checks and first-difference reporting are the skeleton for
    the QEMU oracle comparator;
  - the service order in the `step` loop.
- **DROP:** everything else, including `draw_parent` (28k) and `completed_*`
  (30k).

### App
`sims-app`:
- **PORT:**
  - `owned_worker.rs` (single-owner worker with immutable publication);
  - the `support.rs` shader bridge and pick math;
  - `bin/inspector.rs` as a dev tool.
- **REFERENCE:**
  - the HUD resource-ID-to-position tables (`hud_cluster.rs:150-300`,
    `original_hud.rs:22-102`, `ui_publication/view.rs:632-720`);
  - the tutorial dialog skin and bitmap font flow;
  - the shortcut and drag mappings;
  - the atomic save-write and error classification.
- **DROP:** exploration/startup_play (30k), the window-session recorder and
  native C/ObjC, worker_host, and widget_lifetimes.
- **Framework.** iced fights a game HUD (hard-coded 800×600 `pin` coordinates,
  widget-lifetime workarounds, wall-clock frame stepping). Use winit plus wgpu
  with a fixed-timestep sim thread, and draw the UI as sprite quads with an
  anchor/scale layout layer. Keep iced or egui for dev tools only.

## New decisions this audit raises

1. **Camera rotation is a sim input** (finding 3). Proposed policy, mirroring
   Selected Person ID:
   - each player's camera rotation is per-player sim state, changed by
     commands;
   - a script asking on behalf of a Sim uses the rotation of the player who
     has that Sim selected, falling back to the host's.
   - Zoom and scroll stay purely local.
2. **Text measurement must be deterministic.** Measure with the game's own
   bitmap fonts, or with a bundled font, never with system fonts.
3. **Host timer readings at lot load** (`rng.rs` `load_saved`, nonresidential
   lots) become seeded inputs recorded in the save.
