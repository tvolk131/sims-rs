# Create-a-Family 3D Sim Turntable Preview, Catalog Enumerator, and 3D Portrait Renderer

## Context

Create-a-Family (CAF) in *The Sims: Complete Collection* (1.91) requires three
3D presentation capabilities in `sims-people`:

1. Enumerating selectable head and body outfits filtered by age, gender, and
   skin tone.
2. Rendering the full-body 3D Sim dress-up turntable preview (`cWin3DView`
   inside `cWinDesignCharacter`).
3. Rasterizing 3D Sim head portraits (`BMP_` 2002–2007 stored in
   `UserXXXXX.iff`, plus transparent-background portraits for modern UI
   compositing).

All formulas and constants below were verified directly against `Sims.exe`
1.91 disassembly and live process memory dumps, and cross-checked pixel-by-pixel
against the Complete Collection CAF oracle capture (`User00172.iff` — Alice,
`c809fa_beth,HEAD-HEAD=c809famed_beth3` / `b002fafit_01,BODY=b002fafitmed_mom02`
— and `800×600` CAF screenshots).

## Decisions

### 1. CAF Head & Body Catalog Enumeration (`0x408bf5`, `0x409663`, `0x40a364`)

- **Appearance pool** (`0x409255`): all loaded appearances from `Wardrobe::of(assets)`
  whose names do not start with `"suit-"` (case-insensitive).
- **Texture pool** (`0x40895a`): all `.bmp` texture stems (lowercase, extension
  stripped).
- **Bodies** (`0x409663`): match appearances starting with `b` + 3 arbitrary
  ASCII bytes (`0x47275d(3)` wildcard node) + gender (`m`|`u` for male, `f`|`u`
  for female) + age (`a` for adult, `c` for child) + 3 arbitrary bytes (body
  build tag: `fit`, `fat`, `skn`, `chd`). For each matched 9-byte prefix `P`,
  pair the appearance with every texture stem beginning with `P + skin_tone`
  (`lgt`, `med`, `drk`) as `BODY=<texture>`.
- **Heads** (`0x40a364`): match appearances starting with `c` + 3 arbitrary
  bytes + gender (`m`|`u` or `f`|`u`) + age (`a` or `c`), rejecting any
  appearance whose second byte is `'t'`/`'T'` (`0x40a450`–`0x40a461`, which
  filters out NPC/townie `ct*` heads). For each matched 6-byte prefix `P`, pair
  the appearance with every texture stem beginning with `P + skin_tone` as
  `HEAD-HEAD=<texture>`.
- **Default CAF open hands** (`0x40b050`–`0x40b40a`): `hulo`/`huro` for
  children, `hflo`/`hfro` for adult females, and `hmlo`/`hmro` for adult males.
  When loading via `SimConfig::load_body` / `resolve_hands`, if the selected
  body texture carries a suffix after byte 12 (e.g. `b024masknlgt_mads` →
  `_mads`) and `huao<tone><suffix>.bmp` exists in `Assets`, the body-specific
  hand texture is chosen (matching native longest-match hand selection);
  otherwise it falls back to `HAND=huao<tone>`.
- **Known approximations vs `Sims.exe`**: catalog enumeration inherits
  lexicographic ordering and BMP-only texture indexing from `Wardrobe::of`
  (stock Complete Collection has no CAF-matching TGA stems; native registry
  load order differs from lexicographic order), and prefix offsets use UTF-8
  rather than Windows-1252 single-byte indexing (identical for all ASCII
  stock names).

### 2. 3D CAF Turntable Preview (`cWin3DView`, `0x56cbfb`–`0x56d039`)

- **Viewport & camera** (`0x546459`–`0x546486`): `100×220` (`0x64 × 0xdc`)
  widget positioned at `(618, 145)` (`[0x6f5050]`, `[0x6f5054]`) inside the
  `800×600` CAF screen. `cWinDesignCharacter` sets the house camera to rotation
  `0` (`0x4a7dfe`) and `Sims.exe` zoom `1` (`sims_render::Zoom::Far`,
  `tile_width = 16 << 1 = 32`, `0x4a7b92`), and constructs `cWin3DView` with
  model scale `5.0` (`[0x667678]`). This yields
  `5.0 / (3√2 / 32) = 80√2 / 3 ≈ 37.712` screen pixels per skeleton unit.
- **Anchor & projection** (`0x56cf13`–`0x56d039`): the model transform is
  `RotY(theta) * Scale(5.0) * Translate(anchor)` where `anchor` unprojects
  screen `(width/2, height - 20)` (`rect.bottom - 0x14`), shifted by `(+0.5, +0.5)`
  for Direct3D 6 integer pixel centres. Composed with the house camera at
  rotation `0` (`RotY(3π/4) * RotX(-π/6)`), the view rotation is
  `RotY(theta + 3π/4) * RotX(-π/6)`.
- **Sinusoidal turntable sweep** (`0x56cd1f`–`0x56cd5d`, `0x473424`–`0x4734e2`):
  `theta(t) = (π/4) * sin(2π * t / 10000) + π/4`, sweeping smoothly across
  `[0, π/2]` around `+45°` (facing straight towards the viewer) with a 10-second
  period.
- **Lighting** (`0x56ceb2`): ambient light set to full white `0x00ffffff`
  (`1.0`), textured meshes quantized through `Packed565` and half-up 8-bit
  modulation rounding (`D3D6Target`).

### 3. 3D Sim Portrait Renderer (`0x4e77c6` & `BMP_` 2002–2007)

- **Head parts & projection** (`0x4e810d`, `0x4e7923`–`0x4e7ac6`): collects all
  non-censored parts bound to `"HEAD"` in bind pose with identity bone
  transforms (`0x599bf2(1)`), and rasterizes them into a `200×200` (`0xc8 × 0xc8`)
  supersampled 16-bit 5-6-5 buffer at `SCALE_PIXELS = 80√2 ≈ 113.137` pixels
  per HEAD-bone unit (`30.0 / (1 << zoom)` divided by `3√2 / (16 << zoom)`,
  zoom-invariant) under
  `RotY(0.05π + pitch) * RotZ(π/2) * RotY(yaw + π) * RotX(-π/6)` with full white
  ambient `(1.0, 1.0, 1.0, 1.0)` (`0x4e7ad6`–`0x4e7b02`).
- **Crop & aspect-ratio fitting** (`0x4e7cba`–`0x4e7e1c`): uses either the
  native fixed crop `[76, 45, 122, 125]` (`[0x4c, 0x2d, 0x7a, 0x7d]`) for
  `BMP_` 2002, 2003, 2004, and 2007, or a tight non-background auto-scan
  (`0x4c357c`) for `BMP_` 2005 and 2006; then expands symmetrically inside
  `200×200` to match the target aspect ratio (e.g., `[65, 45, 133, 125]` for
  `35×41` and `[59, 45, 139, 125]` for `40×40`).
- **Two-pass separable cubic Hermite downsampler** (`0x4ba79c`–`0x4bb4a0`):
  kernel `f(t) = (2|t| - 3)t² + 1` (`|t| < 1`), mirror boundary reflection,
  normalized weights, horizontal pass then vertical pass. For native 5-6-5
  `BMP_` parity (`quantize_565: true`), each pass unpacks 5-6-5 pixels to 8-bit
  `(r5 << 3, g6 << 2, b5 << 3)`, adds `+0.5`, truncates (`ftol`), and repacks
  to 5-6-5 (`r8 >> 3, g8 >> 2, b8 >> 3`). For transparent UI portraits
  (`background: None`), filtering runs in premultiplied RGBA so no background
  fringe bleeds into the head silhouette.

## Oracle Parity Results

Verified against Complete Collection `User00172.iff` (Alice) across all six
native `BMP_` portrait chunks:

| Chunk | Dimensions | Mean Abs Error (0–255) | Exact Pixel Match | Within ≤ 8 RGB |
| --- | --- | --- | --- | --- |
| `BMP_` 2002 (`faces`, 3-frame strip) | `105×41` | `0.005` | `99.9%` | `100.0%` |
| `BMP_` 2003 (`rel. images`, 5-frame strip) | `200×40` | `0.002` | `99.9%` | `100.0%` |
| `BMP_` 2004 (`newimage`, 1 px blue border) | `25×25` | `0.011` | `99.8%` | `100.0%` |
| `BMP_` 2005 (`speech_med`, auto-crop) | `16×16` | `0.000` | `100.0%` | `100.0%` |
| `BMP_` 2006 (`speech_large`, auto-crop) | `32×32` | `0.000` | `100.0%` | `100.0%` |
| `BMP_` 2007 (`web_image`, `35×41` at `(5,2)`) | `45×45` | `0.022` | `99.8%` | `100.0%` |

Verified Alice's `100×220` CAF turntable preview against `05-caf-sim-filled.png`
and `06-caf-sim-traits.png` widget rectangle `(618, 145)..(718, 365)`: vertical
framing `y = 16..210` (`195 px` tall, feet at `y = 210` vs oracle `y = 211`) and
width `56 px` (`x = 22..77` vs oracle `x = 19..80`).

## Scope & Boundaries

- Pure presentation module inside `crates/sims-people/src/caf_preview.rs`,
  exported from `crates/sims-people/src/lib.rs`.
- Reuses `sims-people`'s existing `Assets`, `Wardrobe`, `Body`, `Outfit`,
  `Pose`, `Clip`, `skin`, `Packed565`, and `draw_shaded_culled` without
  modifying `sims-render`, `sims-game`, `sims-ui`, or `sims-vm`.
