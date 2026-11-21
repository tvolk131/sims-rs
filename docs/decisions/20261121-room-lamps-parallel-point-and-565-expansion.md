# Room lamps are Direct3D 6 parallel point lights and 5-6-5 skin texels expand by MSB bit-replication

*Decided 2026-11-21.*

Closes the remaining unverified lighting and 5-6-5 texel expansion assumptions from
`20261118-standard-light-shines-from-viewers-side-lamps-light-people.md` ("That the
lamps' lights are directional, not point lights") and
`20261121-people-d3d6-pixel-centers-and-565-modulation.md` ("Direct3D 6's internal
5-6-5 texel expansion and modulation rounding live inside Direct3D's rasterizer").

## Disassembly & Direct3D 6 findings (`Sims.exe`)

- **Room lamps are created as `D3DLIGHT_PARALLELPOINT` (`dltType = 4`), not directional lights.**
  In `0x4673d0`, the house allocates a pool of six room lamp lights via `0x5d009a` →
  `0x5d0d0b` → `0x5d09a0(4)` (storing `dltType = 4 = D3DLIGHT_PARALLELPOINT` at
  `[light + 0x2c]`, with `dwFlags = 3` at `[light + 0x74]`, `dvAttenuation0 = 1.0`,
  `dvAttenuation1 = dvAttenuation2 = 0.0`, `dvRange = sqrt(FLT_MAX)`). By contrast,
  `stdlight.lgt` is created via `0x5d0cd7` → `0x5d09a0(3)` (`D3DLIGHT_DIRECTIONAL`).
- **How `Sims.exe` updates room lamps (`0x4b4b04`..`0x4b4c95`) and how Direct3D 6 lights them.**
  For each lit lamp in the person's room (up to six), `0x4b4ba6`..`0x4b4c18` computes
  squared distance `d²` between the person's root and the lamp position (5 units above
  ground, `0x467f56`), evaluates strength (`1.0` for `d² < 225`,
  `1 / ((d² - 225) * 0.5 + 1)` up to `400`, off past `400`), calls `0x5d0b7c`
  (`dvPosition = lamp_pos`, `dvDirection = normalize(person_root - lamp_pos)`),
  scales colour `(1, 251/255, 209/255)` by strength (`0x5d0c86`, `0x5d0c2d`), and
  adds the light (`0x5ceded`). Because `dltType` is `D3DLIGHT_PARALLELPOINT`,
  Direct3D 6 ignores `dvDirection`, transforms `dvPosition` into camera/view space
  by the full view matrix (`P_view = M_view * [dvPosition, 1]`), and takes the
  parallel light vector towards `P_view` from the view-space origin (the camera eye):
  `L_view = normalize(P_view)`, which in world/drawing coordinates is
  `normalize(lamp_pos - camera_eye)`.
- **Exact house camera eye (`0x49b0de` → `0x5d0595`).**
  `0x49b0de` constructs the orthographic house camera with
  `h = (255 / (2√6)) * 3 + 68`, `far = √3 * (√2 * 3 * 64) / 2 + h / 2`,
  `eye_dist = far - 12.5 ≈ 334.7285`, and `scale_px = 3√2 / (16 << zoom)`. Starting
  at the rotated lot corner (`(0,0,0)`, `(192,0,0)`, `(192,0,192)`, `(0,0,192)` in
  Sims's world frame for rotations 0..3) offset `eye_dist` along the 30°-down
  isometric axis, it shifts along screen right and up by
  `((width >> 1) - scroll_x) * scale_px` and `((height >> 1) - scroll_y) * scale_px`
  (`0x49b3cf`..`0x49b4be`, with `scroll_x = camera.origin[0] + 1`,
  `scroll_y = camera.origin[1]`).
  This also explains the 8:00 AM rotation-0 observation in
  `20261118-standard-light-shines-from-viewers-side-lamps-light-people.md`: because
  `lamp_pos - camera_eye` points away from the viewer, Mortimer's viewer-facing back
  receives no lamp contribution while sides facing away from the viewer receive warm
  fill light.
- **5-6-5 texel expansion by MSB bit-replication before `D3DTOP_MODULATE`.**
  `Sims.exe` uploads skin BMPs into 16-bit 5-6-5 textures by truncation (`0x49f0ae`:
  `r5 = r >> 3`, `g6 = g >> 2`, `b5 = b >> 3`). When Direct3D 6 samples a 5-6-5
  texture for `D3DTOP_MODULATE`, it expands each channel back to 8-bit unorm by MSB
  bit-replication (`(r5 << 3) | (r5 >> 2)`, `(g6 << 2) | (g6 >> 4)`,
  `(b5 << 3) | (b5 >> 2)`) so maximum 5-6-5 white `(31, 63, 31)` maps to `255`
  rather than `248`. At full light (`level = 1.0`), bit-replicated values quantize
  back to the exact same 5-6-5 output; in dim rooms (`level < 1.0`, such as room 4
  at `162/255`), zero-filled masking (`r & 0xf8`) darkened every modulated skin
  texel by up to 3%.

## Changes

- **`View::eye` (`crates/sims-people/src/lot.rs`):** Computes the exact `0x49b0de`
  house camera eye position in the drawing frame for any viewport `(width, height)`,
  zoom, rotation, and scroll origin.
- **`lot::lamps` & `People::draw` (`crates/sims-people/src/lot.rs`):** Takes
  `eye: Vec3` (computed once per frame via `view.eye(target.size())`) and sets each
  room lamp's direction to `(light - eye).normalize()` while keeping strength
  computed from the squared distance between the person's root `at` and `light`.
- **`Packed565` / `expand_565` (`crates/sims-people/src/lot.rs`):** Replaces
  zero-filled `mask_565` with MSB bit-replicated `expand_565` on textured lot draws
  while keeping the shared `Body` cache (used by `People::posed`) and untextured
  meshes (such as the plumb bob) untouched.

## Measured (Screenshot Ratchet over `4406ed18`)

| Scene | `frame` | `surface` | `objects` | `people` |
|---|---|---|---|---|
| `near-r2-0712-bench` | 99.26% | 99.67% | 99.67% | 46.16% |
| `near-r2-0805` | 98.85% → **99.32%** (+0.47) | 99.59% | 99.89% | 42.16% → **73.66%** (+31.50) |
| `far-r0-cut-0816` | 99.52% | 99.60% | 99.13% | 0.77% |
| `far-r1-cut-0816` | 99.23% | 99.32% | 99.20% | 5.61% → **9.36%** (+3.75) |
| `medium-r1-cut-0816` | 99.49% → **99.50%** (+0.01) | 99.66% | 99.99% | 5.51% → **9.28%** (+3.77) |

Zero regressions across all eight scenes and all classes. On `near-r2-0805`, where
Bella and Mortimer stand in lit room 4 (`level = 162/255`, two lit lamps giving
three room lights):

- **Bella (`id=18`):** 1,725/3,376 (51.10%) → **3,364/3,376 (99.64%)** (+1,639 exact 5-6-5 pixels; 8 of the 12 remaining misses are within 1 LSB).
- **Mortimer (`id=21`):** 897/1,416 (63.35%) → **1,410/1,416 (99.58%)** (+513 exact 5-6-5 pixels; 4 of the 6 remaining misses are within 1 LSB).
- Nearly all remaining `people` misses in `near-r2-0805` belong to Cassandra
  (`id=16`), whose animation pose differs between our run and the frozen screenshot.

## Open / unverified

- The remaining 18 pixel differences across Bella and Mortimer on `near-r2-0805`
  (12 within 1 LSB of 5-6-5) are likely minor Gouraud/modulation rounding or
  sub-texel UV differences inside Direct3D 6's rasterizer.
- Room lamp attenuation across multiple rooms and upstairs lamps (5 units above
  ground, below an upstairs person's root) have not been tested against a dedicated
  upstairs screenshot.
