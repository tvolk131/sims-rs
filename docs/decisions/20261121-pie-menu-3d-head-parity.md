# Pie-menu 3D head rendering and backdrop parity

*Decided 2026-11-21.*

Supersedes the placeholder pie-head camera, scale, and lighting in
`20261115-pie-menu-shows-sims-head-drawn-front.md`.

`Sims.exe` draws the pie menu's circular backdrop and 3D head directly in the
house viewport's pass (`0x4a94ec` → `0x4ac8c1` → `0x4ac8ff` for the backdrop,
then `0x4a953e` / `0x4ab77d` → `0x587cf1` → `0x462290` for the 3D head),
clipped to the house viewport before the 2D UI draws the bottom control panel
and pie labels (`0x5eb04c`).

## Verified native behavior (`Sims.exe` Complete Collection 1.91)

- **Parts & bind pose (`0x462437`–`0x462575`, `0x599bf2(1)` → `0x59a020`):**
  Every non-censored part (`censor == 0`) bound to `"HEAD"` (`0x6c4528`) is
  skinned in bind pose with identity bone transforms (`0x6f7620` zero
  translation, `0x6722b8` identity 3×3 rotation), rather than the Sim's live
  lot animation pose.
- **Position & zoom-invariant scale (`0x4625ae`–`0x462685`, `0x49b0de`):**
  In steady state (after the 400 ms pop-in), the HEAD bone origin sits 14 px
  below the main pie menu's center (`0x4625ae`: `0xe`), unprojected into the
  house camera's world space and scaled by `(12.0 / (1 << zoom)) * 1.75`
  (`0x4625d9`, `[0x6c470c] = 1.75`). Because the house camera (`0x49b0de`) has
  `3√2 / (16 << zoom)` world units per pixel, `(1 << zoom)` cancels identically
  across all three zooms, giving a constant orthographic screen scale of
  `56√2 ≈ 79.196` pixels per HEAD-bone unit.
- **Orientation, pointer tracking, and rotation invariance
  (`0x46234d`–`0x462428`, `0x462688`–`0x462778`, `0x462822`–`0x46292d`):**
  `cTSPieMenu::HitTest` (`0x5eb7c9`–`0x5eb8c9`) stores pointer `distance`
  (`+0x164`) and `angle = atan2(dx, -dy)` (`+0x168`, zeroed inside the 35 px
  dead zone when no label box is hit; when `distance < 1`, `0x5eb833` returns
  early without modifying the previous angle). `0x462290` computes
  `r = sqrt(distance * 0.0005)`, `pitch = cos(angle) * r + 0.3/60`,
  `yaw = -sin(angle) * r + 0.3/60`, and adds the 8-way compass offset of the
  highlighted entry (`±0.6` cardinals, `±0.4` diagonals, jump table
  `0x462a53`). The model rotation
  `RotY(0.05π + pitch) * RotZ(π/2) * RotY((rot - 0.5)*(-π/2) + yaw)` composed
  with the house camera view rotation `RotY(-(45° - 90°·rot) + π) * RotX(-π/6)`
  cancels `rot` identically across all four lot rotations, leaving
  `RotY(0.05π + pitch) * RotZ(π/2) * RotY(yaw + π) * RotX(-π/6)`.
- **Lighting & 5-6-5 textures (`0x462576`–`0x4625a4`, `0x4b4cf5`):**
  Every lot person removes its directional and room lights in `0x4b4cf5` before
  `0x587cf1` runs, so the pie head renders with ambient-only lighting: `1.0`
  when an option is highlighted and `0.8` (`[0x667454]`) when idle, using the
  same 5-6-5 texel expansion (`Packed565`) and Direct3D 6 integer pixel-centre
  offset `(+0.5, +0.5)` as lot people.
- **Submenus (`0x462106`, `0x46213e`, `0x4622b6`–`0x46234a`):**
  Opening a submenu moves the dark radial backdrop's target center to the
  submenu (`[esi + 4]`), while the 3D head stays at the parent menu's center
  (`[esi]`) and keeps reading the parent menu's frozen look vector.
- **Dark radial backdrop (`0x4ac8c1` → `0x4ac8ff`):**
  Draws a 100 px radius (`[0x6c4708]`) circular halo in the house viewport over
  every pixel with 16-bit depth `> 0` (`0x4ac8e7` threshold `0`, `0x4aca76`–
  `0x4aca7a`, including empty-depth pixels at `EMPTY = 65535`): inside radius
  `67` (`radius - radius / 3`), pixels desaturate to
  `grey = (r + g + b) / 6 + 8` (reading 5-6-5 zero-filled channels via
  `0x438ef6`); between `67` and `100`, they blend linearly in squared distance
  back to the lot colour and quantize to 5-6-5 (`0x49f0ae`).

## Implementation & deferred animations

- `sims_people::pie_head` implements `Look`, `compass_direction`, `project`,
  `screen_depth_scale`, `depth_clip_bounds`, `draw_body` (with `MAX_SIDE = 4096`
  bounds validation), and `draw_body_in_viewport` (rasterizing at exact
  house-viewport coordinates, clipping to the active house viewport and the
  native Direct3D 6 orthographic near/far frustum `[0.0, 65535.0]` for the
  current zoom at `0x49b506`, `0x4623f5`–`0x462414`, `0x49b56d`, and recording
  per-pixel 16-bit screen depths so oversized headwear such as the stock
  Flamingo hat is never cropped to the 201×201 face bitmap while still matching
  near-plane clipping and `D3DCMP_LESSEQUAL` house depth testing at `0x4a9500`,
  `0x5ce959`–`0x5ce977`), exposed on `sims_people::People::pie_head` and
  `pie_head_in_viewport`.
- `sims_ui::pie::Pie::draw` draws `head` only once on `layouts[0]` so submenus
  do not duplicate the head.
- `sims_game::menu::Menu` tracks `head_look` (freezing the parent look while a
  submenu is open), `main_center`, `active_center`, `pending_pointer`, and
  `head_origin`; opening a pie menu seeds the pointer from the press or
  on-screen clamp warp (`0x5ebd05`, `0x5ebd29`) so the opening frame does not
  aim the head at a stale hover coordinate; `Hud::hover` requests a redraw on
  pointer motion while a pie menu is open; `Game::draw` applies
  `shade_pie_face` and `composite_pie_head` (from `head_in_viewport`) inside
  the house viewport before `Hud::draw` draws the bottom control strip and
  `Menu::draw_labels` draws the 2D pie labels on top.
- **Deferred animations (steady-state rendered):** the three 400 ms transition
  animations are not ported yet:
  1. Backdrop pop-in and submenu/back center-and-radius linear interpolation
     (`0x462106`, `0x46213e`, `0x46216e`, `[0x6c4700] = 400 ms`, interpolating
     from radius `0` on initial open and between parent/submenu centers).
  2. Head pop-in scale and `Y_OFFSET` linear ramp (`0x4625e1`–`0x46260d`,
     `[0x6c4700] = 400 ms`).
  3. Direction-change ease-out (`0x46277a`–`0x4627d2`, `[0x6c4704] = 400 ms`,
     `sqrt(0.1 + 0.9 * t)`).

## Verification against Complete Collection 1.91 captures

Measured over our rendered Mortimer Goth head's opaque pixel mask against real
`Sims.exe` captures:

- `choice2-a.png` (main menu, `Have A Snack` up): **1,749 / 1,750** opaque
  head pixels 5-6-5 exact (**99.94%**, MAE 0.51).
- `talk-submenu.png` (parent menu just before the `Talk...` submenu draws,
  `Talk...` down): **1,828 / 1,830** opaque head pixels 5-6-5 exact
  (**99.89%**, MAE 0.47).
- `read-queued.png` (main menu, `Read a Book` down): **1,826 / 1,830** opaque
  head pixels 5-6-5 exact (**99.78%**, MAE 0.43).
- `social-queued.png` (composed `Talk...` submenu, with `About Interests`
  overlapping the top of the frozen parent head): **806 / 807** opaque head
  pixels outside the overlapping label 5-6-5 exact (**99.88%**).
- Steady-state circular backdrop (`0x4ac8ff`): **9,308 / 9,308** sampled
  static shaded pixels in the paired captures matched exact in 5-6-5.
