# Lot people back-face culling (`D3DCULL_CCW`) and `near-r2-0712-bench` tick alignment

*Decided 2026-11-21.*

Audits every remaining low `people` column across the eight screenshot-ratchet scenes against
`Sims.exe` and the frozen reference PPMs, enables Direct3D 6's default `D3DCULL_CCW` back-face
culling on lot-drawn people and plumb bobs, and aligns `near-r2-0712-bench` to its matching
replay tick (`1255`, still 7:12 AM).

## Disassembly and forensic findings (`Sims.exe`)

- **Back-face culling (`D3DCULL_CCW`).** `Sims.exe`'s Direct3D 6 render-state setup
  (`0x5ce911`–`0x5cea52`) and indexed mesh draw routines (`0x5d34c6`–`0x5d37f1` and
  `0x5d38d5`–`0x5d3c69`) never call `IDirect3DDevice3::SetRenderState` with
  `D3DRENDERSTATE_CULLMODE (22)` (`0x16`); the call at `0x5ce97d`–`0x5ce989` sets
  `D3DRENDERSTATE_FILLMODE (8) = D3DFILL_SOLID (3)`. Direct3D 6 therefore keeps its default
  `D3DCULL_CCW` on the reviewed Sim and plumb-bob draw paths, matching `pie_head.rs`. Enabling
  `D3DCULL_CCW` in `lot.rs` removes back-face overdraw on hair/skin boundaries (for example,
  `+3` exact pixels on Bella in `near-r2-0805`, raising her to 3,367/3,376 = 99.73%, and `+1`
  exact pixel on Cassandra in `near-r2-0712-bench`, raising her to 1,481/1,485 = 99.73%) with
  zero regressions on any tested Sim (and on the closed plumb-bob mesh: 558/558 pixels
  unchanged in `near-r2-0712-bench`, with only grazing back-face pixels culled at rotation 1 in
  `far-r1-cut-0816` and `medium-r1-cut-0816`).
- **No Sim drop-shadow disc.** Verified in both `Sims.exe`'s Sim draw path (`0x4a9411`–`0x4a9544`,
  `0x4ab56d`–`0x4ab727`, `0x590c05`–`0x590f54`) and all five reference screenshots (including
  feet on bright porch tiles and grass): the original does not draw a ground shadow disc under
  Sims.
- **Child skeleton, sitting animations, and carried props match at replay tick `1255`.** At
  `near-r2-0712-bench` replay tick `1255` (7:12 AM), both Sims align with the frozen reference
  screenshot (`0` non-person pixel diffs anywhere in the `(240..440, 250..410)` region spanning
  both Sims and the bench, confirming zero silhouette misses outside our canvas IDs):
  - Cassandra (`id = 16`, `child` skeleton, `C2O-sit-front` + `c2o-rarm-carry-loop` +
    `suit-book101`): **1,481 / 1,485 (99.73%)**.
  - Mortimer (`id = 21`, `adult` skeleton, `A2O-bookshelf-read-sitting-loop1` + `suit-book103`):
    **1,847 / 1,854 (99.62%)**.
  - Combined `people`: **3,328 / 3,339 (99.67%)**, and `objects` (the bench drawn after the
    seated Sim) reaches **100.00%**.
  The previously pinned replay tick `1260` (also 7:12 AM) was five ticks past the matching pose:
  Cassandra had already rotated 90° further into `C2O-sit-front` (`phase 0.4616` vs `0.3462`),
  accounting for nearly all of the former 46.16% `people` score and 99.72% `objects` score.
- **Skinning and quaternion interpolation verified.** `Sims.exe`'s mesh skinning (`0x59a020` ->
  `0x59a4e9` -> `0x59a55a` -> `0x49b698`) blends vertex positions and normals linearly and
  renormalizes blended normals (`skin.rs`), and keyframe quaternion blending (`0x593989`) uses
  normalized linear interpolation (`math.rs`).
- **Remaining soft `people` scenes:**
  - `near-r2-0805` (tick `2854`): Bella (`99.73%`) and Mortimer (`99.58%`) already match at tick
    `2854`, while Cassandra (`id = 16`, walking `c2o-walking-loop` at speed `1.4`) shows two
    separate differences:
    1. *Replay/simulation timing drift:* her captured position in `near-r2-0805.ppm` matches our
       replay at tick `2855` (one tick ahead of Bella and Mortimer's tick `2854`).
    2. *Animation intra-frame interpolation flag (deferred):* disassembly of `cAnimLayer::Update`
       (`0x597f1b`–`0x597f38`) shows `Sims.exe` disables intra-frame interpolation when
       `delta_clock >= 2 * (duration / frames)` (`47.62 ms <= 50 ms` at speed `1.4` on
       `c2o-walking-loop`), sampling the integer keyframe `floor(phase * frames)` directly when
       no frame-skip clamping applies. Our clip sampler (`crates/sims-people/src/animation.rs`)
       still interpolates fractional phases unconditionally because `sims_sim::animator::Layer`
       does not yet store that per-layer flag. Combining tick `2855`'s position with exact
       integer keyframe `10` (`phase = 10/27`) raises Cassandra to **1,924 / 1,945 (98.92%)**.
  - `far-r0-cut-0816`, `far-r1-cut-0816`, and `medium-r1-cut-0816` (tick `3189`): as noted in
    `20261120-screenshot-ratchet.md`, the 8:16 live capture diverged from our replay (about 22
    ticks / 62 px displacement on walking Mortimer at medium zoom). Fitting Mortimer's captured
    pose directly gives **993 / 1,039 (95.57%)** in `medium-r1-cut-0816` and **249 / 258 (96.51%)**
    in `far-r1-cut-0816` at `(0, 0)` pixel shift, confirming far/medium projection, scale, and
    lighting are sound.

## Changes

- **`crates/sims-people/src/lot.rs`:** Switched `draw_lot_part` and the plumb-bob draw in
  `People::draw_in_viewport` from unculled `draw_shaded` to `draw_shaded_culled` (`D3DCULL_CCW`),
  and added unit test `draw_lot_part_culls_counter_clockwise_back_faces`.
- **`crates/sims-people/src/raster.rs`:** Corrected the `draw_shaded_culled` doc comment to cite
  Direct3D 6's un-overridden `D3DRENDERSTATE_CULLMODE` default (`0x5ce911`–`0x5cea52`,
  `0x5d34c6`–`0x5d37f1`, `0x5d38d5`–`0x5d3c69`) and added unit test
  `draw_shaded_culled_drops_counter_clockwise_back_faces`.
- **`crates/sims-people/tests/lot.rs`:** Cleared the lot depth buffer on the `over` comparison
  canvas (`over.depth.fill(sims_render::depth::EMPTY)`) and drew via `draw_on_lot` in both
  `the_lot_hides_the_people_behind_it` and `a_person_upstairs_stands_on_the_upper_floor` instead
  of passing a flat `-1.0e9` depth (which `Canvas::put` clamps to `0`, flattening per-person
  self-occlusion).
- **`crates/sims-game/tests/ratchet.txt`:** Updated `near-r2-0712-bench` from `tick = 1260` to
  `tick = 1255`, re-pinned changed `ours` hashes, and raised floors via `SIMS_RATCHET_UPDATE=1`
  (zero floors lowered).

## Measured (Screenshot Ratchet)

| Scene | `frame` | `surface` | `objects` | `people` | `panel` |
|---|---|---|---|---|---|
| `near-r2-0712-bench` | 99.37% → **99.97%** (+0.60) | 99.81% → **99.98%** (+0.17) | 99.72% → **100.00%** (+0.28) | 46.16% → **99.67%** (+53.51) | 99.93% |
| `near-r2-0805` | 99.48% | 99.81% | 99.94% | 73.66% → **73.70%** (+0.04) | 99.99% |
| `far-r0-cut-0816` | 99.79% | 99.84% | 99.89% | 0.77% | 99.88% |
| `far-r1-cut-0816` | 99.86% | 99.91% | 99.88% | 9.36% | 99.88% |
| `medium-r1-cut-0816` | 99.55% | 99.72% | 99.99% | 9.28% | 99.88% |
| `near-r3-down-0816` | 99.77% | 99.69% | 99.93% | — | 99.88% |
| `near-r0-up-0816` | 99.98% | 99.99% | 100.00% | — | 99.88% |
| `night-far-r0-cut-2153` | 99.90% | 99.98% | 100.00% | — | 99.19% |

`far-r1-cut-0816`, `medium-r1-cut-0816`, and `night-far-r0-cut-2153` also update their `ours`
hashes under `D3DCULL_CCW` (grazing back-face pixels on the left-out plumb bob at rotation 1
and on Sims inside the left-out house region at 9:53 PM); all scored classes in those scenes
remain identical.
