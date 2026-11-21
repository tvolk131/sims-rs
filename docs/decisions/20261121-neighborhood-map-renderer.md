# Neighborhood map screen renderer

## Decision

Add a self-contained, CPU-side Complete Collection 1.91 neighborhood map
renderer and silhouette hit-tester in
`crates/sims-render/src/neighborhood.rs`, covering all six map screens that
share the native `cWinLot` (`0x54dd4d`, vtable `0x670204`) widget pipeline:

- **Unleashed** (`cWinNeighborhood`, `0x556d78..0x55724a`, lots from
  `LotZoning.iff` `STR# 1` in file order up to the first entry with `id > 80`
  at `0x557125..0x557128`, anchors from `LotLocations.iff` `STR# 4`)
- **Classic 1.0** (`cWinNeighborhoodVC`, `0x55eb78..0x55eda3`, lots `1..=10`,
  `STR# 1`)
- **Downtown** (`cWinDowntown`, `0x54a7d6..0x54a8e2`, lots `21..=30` in native
  jump-table Z-order `0x54aca8`, `STR# 2`)
- **Vacation Island** (`cWinVacation`, `0x5691a5..0x5692bd`, lots `40..=48` in
  native jump-table Z-order `0x569680`, `STR# 3`, with `VIsland_port.bmp`,
  `VIsland_trees.bmp`, and post-lot water overlays `0x56baeb..0x56bbb7`)
- **Studio Town** (`cWinStudiotown`, `0x564ce0..0x564db2`, lots `81..=89`,
  `STR# 5`)
- **Magic Town** (`cWinMagicland`, `0x551a8a..0x551b5c`, lots `90..=98`,
  `STR# 7`, with post-lot water waves at `(0, 62)` per `0x542560`, `0x554ff6`)

## Verified against `Sims.exe` (Complete Collection 1.91)

- **Viewport framing & clipping (`0x556d78..0x556e23`, `0x5571e4..0x557201`):**
  The map backdrop is always `800×600`. When the viewport exceeds `800×600`,
  the outer `1024×768` frame (`kLargeMask = 5001` or `kLargeMaskSS = 5002`)
  draws at `(0, 0)`, the map centers at `((w - 800) / 2, (h - 600) / 2)`
  (`(112, 84)` at `1024×768`), and every lot draw clips to
  `[ox, oy, ox + 800, oy + 600]`.
- **Two-cell lot thumbnails (`0x4e261f..0x4e2987`, `0x54e35b..0x54e8fb`):**
  `BMP_ 512`/`1024` is the roofs-off interior view and `BMP_ 513`/`1025` is the
  roofs-on view. Cell 0 (idle) composites `512/1024` at `(max(0, -dx), max(0, -dy))`
  under `513/1025` at `(max(0, dx), max(0, dy))`; Cell 1 (hover cutaway) draws
  `512/1024` alone. Short `THMB` chunks (`< 20` bytes) are zero-padded
  (`0x4e279f..0x4e27c6`); missing `THMB` chunks use sentinel `0x7fffffff`
  (`0x4e27da`, centering placement at `0x54ea0c`). The separate badged hover
  surface (`[esi + 0x264]`, `0x54e58b..0x54e680`) composites `BMP_ 513/1025`
  with `kNghVacationLogo` (`5441`) or `kVacationNoVacancy` (`5510`) centered on
  `BMP_ 512/1024`'s dimensions and draws unhighlighted on hover
  (`0x54ec48..0x54ec91`, across all maps for away-on-vacation homes via
  `0x54eb49`, and Vacation Island via `0x54eb5d`), while picking (`0x5e40b4`)
  continues to sample the regular thumbnail `[esi + 0xd8]`.
- **0.5× Hermite downsampler (`0x4ba778` -> `0x4ba79c`):**
  Base lots `House01..House10` lack `BMP_ 1024/1025` and `THMB 1024`. The
  Unleashed map downsamples `BMP_ 512/513` in two separable 5-6-5 passes with
  `dst = ceil(src * 0.5)` (`0x636b98`), Hermite kernel `(2t - 3)t^2 + 1` for
  `|t| < 1`, reflection boundaries, `0xf81f` color-key skipping, and surviving
  weight cutoff `0.46875` (`[0x668820]`), while halving all five `THMB 512`
  signed integers toward zero (`0x4e285a..0x4e288e`).
- **Oracle pixel match:** Against live Complete Collection 1.91 Neighborhood 1
  captures `c2-hood.png` and `i3-hood.png` (`800×600`, wave frame 2
  `NScreen_unleashed_waves002.bmp`) in the map area below the top HUD toolbar
  (`y = 80..600`), **178,134 / 178,134 lot pixels (100.0000%)** across all 41
  lots—including all 10 downsampled base lots (`House01..House10`, both even
  and odd heights)—match exact 5-6-5 RGB. Overall `y = 80..600` matches
  **414,483 / 416,000 pixels (99.64%)**, with the remaining 1,517 non-lot
  pixels accounted for by animated map props (Nessie) and cursor.
- **Water wave layering (`0x5570c1`, `0x54cb21`, `0x56baeb..0x56bbb7`, `0x542560`, `0x554ff6`):**
  Unleashed and Classic draw water waves before lots at `(0, 0)`. Downtown draws
  waves after lots at `(0, 0)`. Vacation draws port (`5442`), trees (`5443`),
  then waves after lots at `(0, 0)`. Magic Town draws waves after lots at
  `(0, 62)`.
- **Lot highlight (`0x54f28f..0x54f4b4`, `0x54f520`, `0x49bbfe`):**
  In-place left-to-right 5-6-5 scanline pass brightening non-colorkey pixels
  through the native YIQ matrix (`y_scale = 1.0`, `y_offset = 0.09` from
  `[0x6703bc]`) and recoloring horizontal silhouette boundary pixels adjacent
  to `0xf81f` in `RGB(242, 255, 29)` (`0x54e91c`) for hover/selection or
  `RGB(0, 0, 255)` (`0x54f520`) for community lots when an Unleashed filter is
  active (`0x55d5e9..0x55d5ff`).
- **Hit-testing (`0x5e4071` -> `0x5dd79a`, `0x5dc83c`):**
  Reverse draw-order scan testing each lot's widget bounds and active
  (possibly highlighted or badged) surface silhouette without applying the
  `+0x230` drawing clip. Inside the clipped map rect, `Canvas::id_at` agrees
  with `hit_test_with_options`.
- **Community filter badges & occupancy (`0x55d5ac..0x55d7cc`, `0x54cd60`,
  `0x56bdc7`, `0x566fa7`, `0x5550da`, `0x410602`, `0x410673`, `0x54f270`):**
  Reads community amenity bitmasks from `SIMI` `globals[37]`, enforces per-map
  filter ranges (`0x01..=0x40` on Unleashed/Downtown/Vacation, `0x01..=0x400`
  on Studio Town, `0x01..=0x1000` on Magic Town), skips residential lots
  `90..=92` on Magic Town, places badges at `(anchor_x - 24, anchor_y - 10)` on
  lots `23, 40, 42, 83` and `(anchor_x - 24, anchor_y - 70)` elsewhere, and
  reads `FAMI` chunk IDs plus version `>= 8` secondary houses from
  `Neighborhood.iff`.

## Deliberate simplifications / out of scope

- Top toolbar chrome, family/lot popup dialogs, hover fade timer (`300 ms`),
  and ambient animated props (Nessie, Studio Town cars, Magic Town balloons
  and mist) belong to UI/screen-controller layers rather than this pure map
  renderer; water wave animation frames are selected explicitly via
  `RenderOptions::water_frame`.
- `RenderOptions::show_occupancy_markers` provides an optional clean indicator
  for residential occupied/vacant lots when a caller wants visual status without
  hovering a lot tooltip; it defaults to `false` so native renders remain exact.
