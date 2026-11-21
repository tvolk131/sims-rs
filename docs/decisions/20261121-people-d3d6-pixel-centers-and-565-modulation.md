# People rasterization matches Direct3D 6 integer pixel centres and 5-6-5 texel modulation

*Decided 2026-11-21.*

Closes the open question from `20261120-mortimer-standing-without-his-book-matches-the-original.md`
("why the original's people land half a pixel right of and below ours") and matches the
original's 16-bit 5-6-5 texture packing and `D3DTOP_MODULATE` rounding for lot-drawn people.

## Disassembly findings (`Sims.exe`)

- **No software triangle rasterizer — Direct3D 6 renders every 3D mesh.** Person and plumb-bob
  meshes are drawn via `IDirect3DDevice3::DrawIndexedPrimitive` (`0x5d3b44`) and
  `DrawIndexedPrimitiveVB` (`0x5d36c6`), which is why `0x5c0000..0x5e0000` has no `fistp`
  rasterizer loop.
- **Integer pixel centres (`0x49b0de`, `0x5cf263`).** The house camera (`0x49b0de`) shifts by
  integer scroll (`0x6f00d0`, `0x6f00d4`) and half viewport width/height (`0x49b42c`, `0x49b462`).
  The viewport matrix (`0x5cf263`) maps NDC `(0, 0)` directly to
  `(vp.dwX + width/2, vp.dwY + height/2)` with no `-0.5` offset. Under Direct3D 6 rasterization
  rules, pixel `(x, y)` has its sample centre at integer `(x, y)`, whereas `sims-people`'s
  `raster::triangle` samples at `(x + 0.5, y + 0.5)`. That shifts every lot-drawn triangle
  `(-0.5, -0.5)` pixels relative to Direct3D 6 — explaining the exact `(+0.5, +0.5)` best fit
  measured across Mortimer (7:12 and 8:05) and Bella (8:05).
- **Direct3D 6 render states and white materials.** Verified in `0x49b07a`, `0x5ce911..0x5cea52`,
  and `0x5d3540..0x5d3c40`:
  - `D3DRENDERSTATE_SHADEMODE (9) = 2` (`D3DSHADE_GOURAUD`, `0x5ce935`)
  - `D3DRENDERSTATE_TEXTUREPERSPECTIVE (4) = 0` (affine UVs, `0x5ce942`)
  - `D3DRENDERSTATE_ZENABLE (7) = 1`, `ZWRITEENABLE (0xe) = 1`, `ZFUNC (0x17) = 4` (`D3DCMP_LESSEQUAL`, `0x5ce959..0x5ce977`)
  - `D3DRENDERSTATE_FILLMODE (8) = 3` (`D3DFILL_SOLID`, `0x5ce989`)
  - `D3DRENDERSTATE_DITHERENABLE (0x1a) = 0` (`0x5ce99e`)
  - `D3DRENDERSTATE_SPECULARENABLE (0x1d) = 0` (`0x5ce9b3`)
  - `D3DTSS_MINFILTER (0x11) = 1` (`D3DTFN_POINT`), `D3DTSS_MAGFILTER (0x10) = 1` (`D3DTFG_POINT`, `0x5d3a65..0x5d3a79`, `0x49b090`)
  - `D3DTSS_COLOROP (1) = 4` (`D3DTOP_MODULATE`), `COLORARG1 (2) = 2` (`D3DTA_TEXTURE`), `COLORARG2 (3) = 0` (`D3DTA_DIFFUSE`, `0x5d3a13..0x5d3a36`)
  - Textured mesh materials are created as white (`0x5d44dc -> 0x5d41db -> 0x5d452b`) and reset
    to white diffuse/ambient `(1, 1, 1, 1)` with zero specular/emissive when untinted (`0x5a1b5c`),
    settling the previously unverified material colour assumption from `20261115` and `20261118`.
- **16-bit 5-6-5 texture surface packing (`0x5ca55f -> 0x5ca635 -> 0x49f0ae`).** When uploading a
  skin BMP to a Direct3D 6 texture surface, `0x49f0ae` packs 24-bit RGB to 16-bit 5-6-5 by
  truncation (`r & 0xf8`, `g & 0xfc`, `b & 0xf8`), dropping the low 3/2/3 bits before
  `D3DTOP_MODULATE` multiplies by the interpolated Gouraud diffuse colour and writes to the
  16-bit 5-6-5 backbuffer with dithering off.

## Changes

- **`People::draw` (`crates/sims-people/src/lot.rs`):** Adds `(+0.5, +0.5)` to projected vertex
  screen positions passed to `raster` so Direct3D 6 integer pixel centres map onto `raster`'s
  half-integer pixel-centre sampler. `View::screen` itself stays in unshifted screen coordinates,
  and offscreen `raster::Canvas` renders (pinned in `goth_family` and `playback`) stay bit-identical.
- **`Target::write_coloured` (`crates/sims-people/src/raster.rs`) & `impl Target for sims_render::Canvas`
  (`crates/sims-people/src/lot.rs`):** Routes both grey and coloured (lamp-lit) shading on the
  lot canvas through one implementation that masks texels to 5-6-5 (`0xf8, 0xfc, 0xf8`, `0x49f0ae`)
  and rounds the 8-bit modulated result `(f32::from(c & mask) * s + 0.5).clamp(0.0, 255.0) as u8`
  before `Canvas` quantizes to 5-6-5 (`DITHERENABLE = 0`). Rounding avoids truncating a 1-ULP
  barycentric shortfall (`w0 + w1 + w2 = 0.99999994`) down by a full 5-6-5 step at full light.

## Measured (Screenshot Ratchet over `4102349d`)

| Scene | `frame` | `surface` | `objects` | `people` |
|---|---|---|---|---|
| `near-r2-0712-bench` | 99.00% → **99.24%** (+0.24) | 99.67% | 99.65% → **99.67%** (+0.02) | 17.16% → **46.16%** (+29.00) |
| `near-r2-0805` | 98.37% → **98.84%** (+0.47) | 99.55% → **99.59%** (+0.04) | 99.79% → **99.89%** (+0.10) | 14.13% → **42.16%** (+28.03) |
| `far-r0-cut-0816` | 99.52% | 99.60% | 99.13% | 1.07% → **0.77%** (-0.30; 2 vs 3 coincidental px of 258 on a misplaced walking Sim) |
| `far-r1-cut-0816` | 99.22% → **99.23%** (+0.01) | 99.32% | 99.17% → **99.20%** (+0.03) | 0.74% → **5.61%** (+4.87) |
| `medium-r1-cut-0816` | 99.48% → **99.49%** (+0.01) | 99.66% | 99.97% → **99.99%** (+0.02) | 1.60% → **5.51%** (+3.91) |

The remaining `people` mismatches in `near-r2-0712-bench` and `near-r2-0805` are dominated by
Cassandra's differing pose/walk phase in those captures and minor indoor lighting/Gouraud-interpolation
differences on Mortimer and Bella.
