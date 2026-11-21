# Neighborhood map screen renderer

## Decision

Add a self-contained, CPU-side Complete Collection 1.91 neighborhood map
renderer and silhouette hit-tester in
`crates/sims-render/src/neighborhood.rs`, covering all six map screens that
share the native `cWinLot` (`0x54dd4d`, vtable `0x670204`) widget pipeline:

- **Unleashed** (`cWinNeighborhood`, `0x556d78..0x55724a`, lots `1..=10` and
  `50..=80` from `LotZoning.iff` `STR# 1` with `id <= 80`, anchors from
  `LotLocations.iff` `STR# 4`)
- **Classic 1.0** (`cWinNeighborhoodVC`, `0x55eb78..0x55eda3`, lots `1..=10`,
  `STR# 1`)
- **Downtown** (`cWinDowntown`, `0x54a7d6..0x54a8e2`, lots `21..=30` in native
  jump-table Z-order `0x54aca8`, `STR# 2`)
- **Vacation Island** (`cWinVacation`, `0x5691a5..0x5692bd`, lots `40..=48` in
  native jump-table Z-order `0x569680`, `STR# 3`, with `VIsland_port.bmp` and
  `VIsland_trees.bmp` overlays)
- **Studio Town** (`cWinStudiotown`, `0x564ce0..0x564db2`, lots `81..=89`,
  `STR# 5`)
- **Magic Town** (`cWinMagicland`, `0x551a8a..0x551b5c`, lots `90..=98`,
  `STR# 7`)

## Verified against `Sims.exe` (Complete Collection 1.91)

- **Viewport framing & clipping (`0x556d78..0x556e23`, `0x5571e4..0x557201`):**
  The map backdrop is always `800×600`. When the viewport exceeds `800×600`,
  the outer `1024×768` frame (`kLargeMask = 5001` or `kLargeMaskSS = 5002`)
  draws at `(0, 0)`, the map centers at `((w - 800) / 2, (h - 600) / 2)`
  (`(112, 84)` at `1024×768`), and every lot draw and hit-test clips to
  `[ox, oy, ox + 800, oy + 600]`.
- **Two-cell lot thumbnails (`0x4e261f..0x4e2987`, `0x54e35b..0x54e8fb`):**
  `BMP_ 512`/`1024` is the roofs-off interior view and `BMP_ 513`/`1025` is the
  roofs-on view. Cell 0 (idle) composites `512/1024` at `(max(0, -dx), max(0, -dy))`
  under `513/1025` at `(max(0, dx), max(0, dy))`; Cell 1 (hover cutaway) draws
  `512/1024` alone. The cell top-left sits at
  `(lot.x + min(0, dx) - anchor_x, lot.y + min(0, dy) - anchor_y - anchor_y_extra)`,
  or centered on `(lot.x, lot.y)` when `THMB` holds sentinel `0x7fffffff`.
- **0.5× Hermite downsampler (`0x4ba778` -> `0x4ba79c`):**
  Base lots `House01..House10` lack `BMP_ 1024/1025` and `THMB 1024`. The
  Unleashed map downsamples `BMP_ 512/513` in two separable 5-6-5 passes with
  `dst = ceil(src * 0.5)` (`0x636b98`), Hermite kernel `(2t - 3)t^2 + 1` for
  `|t| < 1`, reflection boundaries, `0xf81f` color-key skipping, and surviving
  weight cutoff `0.46875` (`[0x668820]`), while halving all five `THMB 512`
  signed integers toward zero (`0x4e285a..0x4e288e`).
- **Oracle pixel match:** Against live Complete Collection 1.91 Neighborhood 1
  capture `c2-hood.png` (`800×600`, wave frame 2 `NScreen_unleashed_waves002.bmp`)
  in the map area below the top HUD toolbar (`y = 80..600`), **178,134 / 178,134
  lot pixels (100.0000%)** across all 41 lots—including all 10 downsampled base
  lots (`House01..House10`, both even and odd heights)—match exact 5-6-5 RGB.
  Overall `y = 80..600` matches **414,483 / 416,000 pixels (99.64%)**, with the
  remaining 1,517 non-lot pixels accounted for by animated map props (Nessie)
  and cursor.
- **Lot highlight (`0x54f28f..0x54f4b4`, `0x49bbfe`):**
  In-place left-to-right 5-6-5 scanline pass brightening non-colorkey pixels
  through the native YIQ matrix (`y_scale = 1.0`, `y_offset = 0.09` from
  `[0x6703bc]`) and recoloring horizontal silhouette boundary pixels adjacent
  to `0xf81f` in `RGB(242, 255, 29)` (`0x54e91c`).
- **Hit-testing (`0x5e4071` -> `0x5dd79a`):**
  Reverse draw-order scan inside the clipped `800×600` map rectangle, testing
  non-colorkey (`!= 0xf81f`) pixels in the active cell. `Canvas::id_at` is
  populated with the winning lot ID during rendering so rendering and hit-tests
  stay in sync.
- **Community filter badges & occupancy (`0x55d5ac..0x55d735`, `0x427f46`,
  `0x54f20e`):**
  Reads community amenity bitmasks from `SIMI` `globals[37]`, maps all 13
  filter bits (`0x0001..=0x1000`) to native 3-frame sprites (`4930..=4942`)
  with ping-pong frame order `[0, 1, 2, 2, 1, 0]` drawn at
  `(offset_x + lot.x - 24, offset_y + lot.y - 70)`, and reads family primary
  and vacation lots from `Neighborhood.iff` `FAMI` chunks.

## Deliberate simplifications / out of scope

- Top toolbar chrome, family/lot popup dialogs, hover fade timer (`300 ms`),
  and ambient animated props (Nessie, Studio Town cars, Magic Town balloons
  and mist) belong to UI/screen-controller layers rather than this pure map
  renderer; water wave animation frames are selected explicitly via
  `RenderOptions::water_frame`.
- `RenderOptions::show_occupancy_markers` provides an optional clean indicator
  for residential occupied/vacant lots when a caller wants visual status without
  hovering a lot tooltip; it defaults to `false` so native renders remain exact.
