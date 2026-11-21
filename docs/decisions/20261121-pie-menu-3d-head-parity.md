# Pie-menu 3D head rendering and backdrop parity

*Decided 2026-11-21.*

Supersedes the placeholder pie-head camera, scale, and lighting in
`20261115-pie-menu-shows-sims-head-drawn-front.md`.

`Sims.exe` draws the pie menu's 3D head and circular backdrop directly in the
house viewport's Direct3D 6 pass (`0x4a94ec` → `0x4ac8c1` → `0x4ac8ff` for the
backdrop, then `0x4a953e` / `0x4ab77d` → `0x587cf1` → `0x462290` for the 3D
head), before the 2D UI draws the bottom panel and pie labels (`0x5eb04c`).

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
  dead zone when no label box is hit). `0x462290` computes
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
- **Submenus (`0x462106`, `0x4622b6`–`0x46234a`):**
  Opening a submenu moves the dark radial backdrop's target center to the
  submenu (`[esi + 4]`), while the 3D head stays at the parent menu's center
  (`[esi]`) and keeps reading the parent menu's frozen look vector.
- **Dark radial backdrop (`0x4ac8c1` → `0x4ac8ff`):**
  Draws a 100 px radius (`[0x6c4708]`) circular halo in the house viewport:
  inside radius `67` (`radius - radius / 3`), pixels desaturate to
  `grey = (r + g + b) / 6 + 8` (reading 5-6-5 zero-filled channels via
  `0x438ef6`); between `67` and `100`, they blend linearly in squared distance
  back to the lot colour and quantize to 5-6-5 (`0x49f0ae`).

## Implementation & verification

- `sims_people::pie_head` implements `Look`, `compass_direction`, `project`,
  and `draw_body`, exposed on `sims_people::People::pie_head`.
- `sims_ui::pie::Pie::draw` draws `head` only once on `layouts[0]` so submenus
  do not duplicate the head.
- `sims_game::menu::Menu` tracks `head_look` (freezing the parent look while a
  submenu is open) and `active_center`; `Hud::hover` requests a redraw on
  pointer motion while a pie menu is open; `Game::draw` applies
  `shade_pie_face` inside the house viewport and renders `menu.head` via
  `People::head_looking`.
- The 400 ms pop-in scale/offset animation (`[0x6c4700]`) and 400 ms direction
  change ease-out (`[0x6c4704]`, `sqrt(0.1 + 0.9 * t)`) are **not** ported yet;
  the head renders in steady state.
- Verified pixel-for-pixel against Complete Collection 1.91 captures of
  Mortimer Goth:
  - `choice2-a.png` (up / `Have A Snack`): 1,749 / 1,750 pixels 5-6-5 exact
    (99.94%, MAE 0.51).
  - `talk-submenu.png` (down / `Talk...`): 1,828 / 1,830 pixels 5-6-5 exact
    (99.89%, MAE 0.47).
  - `read-queued.png` (down / `Read a Book`): 1,826 / 1,830 pixels 5-6-5 exact
    (99.78%, MAE 0.43).
