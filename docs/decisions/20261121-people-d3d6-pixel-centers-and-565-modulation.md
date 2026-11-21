# People rasterization matches Direct3D 6 integer pixel centres and 5-6-5 texel modulation

*Decided 2026-11-21.*

Closes the open question from `20261120-mortimer-standing-without-his-book-matches-the-original.md`
("why the original's people land half a pixel right of and below ours") and matches the
original's 16-bit 5-6-5 skin texture packing on load alongside rounded 8-bit modulation on
lot-drawn people.

## Disassembly findings (`Sims.exe`)

- **Indexed mesh draw paths submit directly to Direct3D 6.** Person (`0x590f4f`) and plumb-bob
  (`0x586423`) meshes draw through `IDirect3DDevice3::DrawIndexedPrimitive` (`0x5d3b44`) and
  `DrawIndexedPrimitiveVB` (`0x5d36c6`), which explains why `0x5c0000..0x5e0000` has no `fistp`
  software triangle rasterizer for these draws.
- **Integer pixel centres (`0x49b0de`, `0x5cf263`).** The house camera (`0x49b0de`) shifts by
  integer scroll (`0x6f00d0`, `0x6f00d4`) and halves the view rectangle with arithmetic right
  shift (`sar` at `0x49b42c`, `0x49b462`). The separate viewport matrix helper (`0x5cf263`)
  uses floating-point half-width and half-height, mapping NDC `(0, 0)` directly to
  `(vp.dwX + width/2, vp.dwY + height/2)` with no `-0.5` offset (exact integers for even
  dimensions such as the 800×600 viewport; an odd dimension can place the centre on a
  half-integer). Under Direct3D 6 rasterization rules, pixel `(x, y)` has its sample centre at
  integer `(x, y)`, whereas `sims-people`'s `raster::triangle` samples at `(x + 0.5, y + 0.5)`.
  That shifts every lot-drawn triangle `(-0.5, -0.5)` pixels relative to Direct3D 6 — explaining
  the exact `(+0.5, +0.5)` best fit measured across Mortimer (7:12 and 8:05) and Bella (8:05).
- **Default Direct3D 6 render states and white materials.** Verified in `0x49b07a`,
  `0x5ce911..0x5cea52`, and `0x5d3540..0x5d3c40`:
  - Defaults set by `0x49b07a` and applied by `0x5ce911..0x5cea52`: `D3DRENDERSTATE_SHADEMODE (9) = 2`
    (`D3DSHADE_GOURAUD`, `0x5ce935`), `TEXTUREPERSPECTIVE (4) = 0` (affine UVs, `0x5ce942`),
    `ZENABLE (7) = 1` / `ZWRITEENABLE (0xe) = 1` / `ZFUNC (0x17) = 4` (`D3DCMP_LESSEQUAL`,
    `0x5ce959..0x5ce977`), `FILLMODE (8) = 3` (`D3DFILL_SOLID`, `0x5ce989`),
    `DITHERENABLE (0x1a) = 0` (`0x5ce99e`), `SPECULARENABLE (0x1d) = 0` (`0x5ce9b3`), and
    default point filtering (`[state + 8] = 1`, `0x49b090`, selecting `D3DTFN_POINT` /
    `D3DTFG_POINT` at `0x5d3a65..0x5d3a79`). The draw routines also contain branches for linear
    filtering (`0x5d3a4b`), texture-stage selection (`0x5d399d`), and alpha-blended draws with
    `ZWRITEENABLE = 0` (`0x5d3aa5`).
  - Textured mesh draws use `D3DTOP_MODULATE` (`COLOROP (1) = 4`, `COLORARG1 (2) = 2`
    `D3DTA_TEXTURE`, `COLORARG2 (3) = 0` `D3DTA_DIFFUSE`, `0x5d3a13..0x5d3a36`).
  - Textured mesh materials are created as white (`0x5d44dc -> 0x5d41db -> 0x5d452b`) and reset
    to white diffuse/ambient `(1, 1, 1, 1)` with zero specular/emissive when untinted (`0x5a1b5c`),
    settling the previously unverified material colour assumption from `20261115` and `20261118`.
  - The plumb bob's `white` material branches directly to `0x5d452b` before texture lookup,
    leaving a zero texture handle (`0x5d48ce`), so it has no packed 5-6-5 texel to modulate.
- **16-bit 5-6-5 texture surface packing (`0x5ca0d3`, `0x5ca1f8`, `0x5ca55f -> 0x5ca635 -> 0x49f0ae`).**
  When uploading skin BMPs to a 16-bit Direct3D 6 texture surface, both the indexed 8-bit paths
  used by the installed Goth skins (`0x5ca0d3`, `0x5ca1f8`) and the 24-bit path (`0x5ca55f`)
  call the packer dispatcher `0x5ca635`, which selects `0x49f0ae` for 5-6-5 destination masks.
  `0x49f0ae` packs 8-bit RGB into 5-6-5 by truncation (`r & 0xf8`, `g & 0xfc`, `b & 0xf8`).
- **Unverified / approximation.** `Sims.exe` verifies `D3DTOP_MODULATE` and disabled dithering,
  but Direct3D 6's internal 5-6-5 texel expansion and modulation rounding live inside Direct3D's
  rasterizer, not `Sims.exe`. Our `sims_render::Canvas` target stores 8-bit RGB in
  `Canvas::color` and quantizes to 5-6-5 in `rgb565()`; its half-up 8-bit rounding
  `(f32::from(c) * s + 0.5).clamp(0.0, 255.0) as u8` is an approximation chosen so 1-ULP
  barycentric shortfalls (`w0 + w1 + w2 = 0.99999994`) do not truncate a full 5-6-5 step.

## Changes

- **`project_lot_vertex` & `People::draw` (`crates/sims-people/src/lot.rs`):** Adds `(+0.5, +0.5)`
  to projected vertex screen positions passed to `raster` so Direct3D 6 integer pixel centres map
  onto `raster`'s half-integer pixel-centre sampler. `View::screen` stays in unshifted screen
  coordinates, and offscreen `raster::Canvas` renders (pinned in `goth_family` and `playback`)
  stay bit-identical.
- **`pack_body_565` & `People::update` (`crates/sims-people/src/lot.rs`):** Truncates textured
  parts (`part.texture_name.is_some()`) to 5-6-5 (`r & 0xf8`, `g & 0xfc`, `b & 0xf8`) on load,
  leaving untextured parts (the plumb bob's `white` material) at full 8-bit white.
- **`Target::write_coloured` (`crates/sims-people/src/raster.rs`) & `impl Target for sims_render::Canvas`
  (`crates/sims-people/src/lot.rs`):** Routes both grey and coloured (lamp-lit and plumb-bob)
  shading on the lot canvas through one half-up 8-bit rounding implementation while preserving
  `raster::Canvas`'s existing offscreen behaviour.

## Measured (Screenshot Ratchet over `4102349d`)

| Scene | `frame` | `surface` | `objects` | `people` |
|---|---|---|---|---|
| `near-r2-0712-bench` | 99.00% → **99.24%** (+0.24) | 99.67% | 99.65% → **99.67%** (+0.02) | 17.16% → **46.16%** (+29.00) |
| `near-r2-0805` | 98.37% → **98.84%** (+0.47) | 99.55% → **99.59%** (+0.04) | 99.79% → **99.89%** (+0.10) | 14.13% → **42.16%** (+28.03) |
| `far-r0-cut-0816` | 99.52% | 99.60% | 99.13% | 1.07% → **0.77%** (-0.30; 3/279 → 2/257, net loss of 1 coincidental px on displaced walking Sims) |
| `far-r1-cut-0816` | 99.22% → **99.23%** (+0.01) | 99.32% | 99.17% → **99.20%** (+0.03) | 0.74% → **5.61%** (+4.87) |
| `medium-r1-cut-0816` | 99.48% → **99.49%** (+0.01) | 99.66% | 99.97% → **99.99%** (+0.02) | 1.60% → **5.51%** (+3.91) |

In `far-r0-cut-0816`, Mortimer's displaced coat lost a coincidental match at `(418, 366)`, while
Cassandra lost `(589, 416)`, gained `(590, 417)`, and kept `(589, 420)`; a people-free control
differs from the screenshot at all four coordinates. In `near-r2-0712-bench`, Cassandra's
differing pose accounts for roughly 90% of remaining `people` misses; in `near-r2-0805`,
Cassandra accounts for roughly 45%, and Bella and Mortimer account for 2,170 of the 3,951
remaining misses (whose remaining causes are not isolated by the ratchet).
