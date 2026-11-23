# ADR: Neighborhood Arbitrary-Viewport Framing & Minimum Window Size

- **Date:** 2026-11-22
- **Status:** Accepted
- **Scope:** `crates/sims-game/src/main.rs`, `crates/sims-game/src/neighborhood.rs`, `crates/sims-render/src/neighborhood.rs`, `crates/sims-ui/src/neighborhood.rs`

## Context

The original Sims 1 Complete Collection supported only two fixed display resolutions (`800×600` and `1024×768`). When `sims-rs` ran inside a freely resizable desktop window, two classes of visual bugs appeared on the Neighborhood screen:

1. **No minimum window size:** `winit` permitted shrinking below `800×600`, crushing the circular top-bar controls into thin vertical slivers and clipping the `800×600` map. On fractional HiDPI displays (`1.5×`, `1.75×`), rounding `scale_factor` up to `2` could also shrink an `800×600` logical window below `800×600` canvas pixels.
2. **Broken `1024×768` outer frame (`large_frame`) at non-native sizes:** `NeighborhoodMap` blitted `Downtown/LargeBack.bmp` (`5001`) or `Studiotown/dlgframe_1024x768.bmp` (`5002`, whose `[112..912, 84..684)` interior is magenta `COLOR_KEY_565`) raw at `[0, 0]` whenever `viewport[0] > 800`, while centering the `800×600` map at `[(vw - 800) / 2, (vh - 600) / 2]`. Wide viewports (e.g. `1440×768`) showed a solid black void past `x = 1024` and a stray misaligned inner cutout shadow on the left; intermediate sizes (`960×720`, `1024×860`) misaligned the cutout; tall narrow sizes (`800×800`) disabled `large_frame` entirely and drew solid black letterbox bars.

## Decision

1. **Minimum logical window size `800×600` (`crates/sims-game/src/main.rs`):**
   - Enforce `.with_min_inner_size(LogicalSize::new(800.0, 600.0))` on `WindowAttributes` (initial size remains `1024×768`).
   - Derive the integer pixel-doubling scale with floor (`(scale_factor + 1e-4).floor().max(1.0)`) so fractional HiDPI scales (`1.25×`, `1.5×`, `1.75×`, `2.5×`) never divide an `800×600` logical window below `800×600` canvas pixels, and snap odd physical boundary rounding right at the `800×600` minimum (`1599×1199 @ 2× → 800×600`).
2. **Bit-exact native fast paths + seamless arbitrary-viewport surround (`crates/sims-render/src/neighborhood.rs`):**
   - `viewport[0] <= 800 && viewport[1] <= 600` (including native `800×600`): unchanged, no outer frame.
   - `viewport == [1024, 768]`: unchanged, `large_frame` blitted 1:1 at `[0, 0]`.
   - Any other viewport where `viewport[0] > 800 || viewport[1] > 600`: both `render_into_with_damage` and `render_into_uncached` call the same `render_outer_frame` helper, which:
     1. Samples the four clean inner-perimeter corner patches of `large_frame` (strictly inside the 16 px bevel and outside the `[112..912, 84..684)` cutout / magenta key) and fills the entire viewport with a smooth bilinear royal-blue gradient (with a matching royal-blue fallback when `large_frame` is absent).
     2. Blits the four `16×16` outer bevel corners 1:1 and stretches the four `16 px` outer bevel edges along their length to hug `(0, 0)..(vw, vh)` cleanly.
     3. Applies a multiplicative drop shadow around the centered map rectangle `[ox..ox+bw, oy..oy+bh)` extracted from `large_frame`'s native cutout shadow attenuation (skipping `COLOR_KEY_565` and feathering smoothly to `1.0` so there is never a hard seam or magenta leak on Studio Town / Magic Town).
   - `CachedBaseLayer` tracks `active_large_frame` whenever `viewport[0] > 800 || viewport[1] > 600` so cached and uncached renders stay 0-diff at every size, and `hit_test_with_options` clips picking to `lot_clip_for(viewport)` on arbitrary resized viewports so the surround never exposes invisible overhanging lot hotspots (preserving native unclipped picking at `800×600` and `1024×768`).
3. **Chrome alignment & defensive `< 800` scaling (`crates/sims-ui/src/neighborhood.rs`, `crates/sims-game/src/neighborhood.rs`, `crates/sims-game/src/main.rs`):**
   - `NeighborhoodShell::resize` and `App::sync_size` re-hit-test any stationary cursor against the resized layout, clearing stale lot highlights/cards when a lot moves away from the cursor while preserving and re-anchoring an open card when the same lot still sits under the cursor.
   - `draw_four_cell_button` preserves sprite aspect ratio when scaling into compact `< 800` width slots while keeping all `>= 800` width layouts untouched.

## Consequences

- `800×600` and `1024×768` Neighborhood renders remain 100.00% bit-exact with Complete Collection 1.91 and all existing ratchets (`neighborhood_map`, `ui_ratchet`, live `ratchet`, and Goth 43,200-tick hash).
- Resizing to any aspect ratio (`800×800`, `960×720`, `1024×860`, `1440×768`, `1920×1080`, etc.) displays a seamless royal-blue beveled frame with a centered recessed map, aligned top bar, and zero black voids or magenta leaks across all six neighborhood kinds.
