# HUD Strip Layout Scaling, `BackPatch.bmp` Wedge Fix, and Native 1.91 Hotkeys

**Date:** 2026-11-22  
**Branch:** `ui/hud-strip-layout-alpha-and-native-hotkeys`  
**Scope:** `crates/sims-ui/`, `crates/sims-game/`

---

## 1. Context

Playtesting the Live Mode bottom HUD and keyboard controls at widescreen resolutions (`> 1024 px`) surfaced three regressions relative to *The Sims Complete Collection* (`Sims.exe` 1.91):

1. **Mood/Tab Gadget Teleporting & Dark Seam at Non-`800`/`1024` Widths:** At any viewport width other than exact `800` or `1024`, the 7-button mood/tab gadget (`id::GAUGE`, `4911`) jumped to the far right (`width - 387`), leaving a wide empty gap between the 4-column Sim portraits (`x = 220..413`) and the gadget. In addition, stretching `PanelBack.bmp` (`804×100`) beyond `1024 px` repeated its rightmost dark bevel column (`x = 803`, RGB `(0, 0, 49)`) across `x = 1024..width`, producing a dark vertical band across the strip.
2. **Gray Translucent Wedge Next to the Camera Button:** When the bottom strip was expanded, a triangular patch of lot background gray (`RGB(115, 113, 115)`) bled through next to the Camera (`PHOTO`) and Options (`...`) buttons (`x = 168..220, y = height - 100..height`) over terrain/grass, while tall world objects extending below `height - 100` could occasionally overlap it.
3. **Non-Authentic Keyboard Hotkeys:** `Space` was bound to pause and `1`/`2`/`3` were bound to wall states instead of authentic Sims 1.91 controls (`` ` `` / `P` pause, `1`/`2`/`3` speed, `Space` next Sim, `Enter` center Sim, `F1`–`F5` + `F10` mode/panel controls, `Home`/`End` wall stepping, `Tab` floor toggle, `C` snapshot, `Z`/`X` Buy & Build catalog paging, and `Ctrl/Cmd+S` save).

---

## 2. Root Causes & Native `Sims.exe` 1.91 Verification

### 2.1 Widescreen Strip Layout & `PanelBack.bmp` Stretching

- **Gadget anchor (`crates/sims-ui/src/hud.rs::live_strip`):**
  - Previously, `extra` was hard-coded as `if strip.x + strip.width == 1024 { 224 } else { 0 }`.
  - In `Sims.exe` 1.91 (`0x518f40..0x518f48`, `0x51f258..0x51f260`, `0x51f527..0x51f52f`), moving from `800` to `1024` keeps `GAUGE` anchored at `x = 413` beside the 4-column portrait block (`220..413`) and gives the entire width increase (`+224 px`) to the active subpanel (`521..width`).
  - Generalizing `extra = (strip.x + strip.width).saturating_sub(800).max(0)` preserves exact `800×600` (`extra = 0`, `subpanel_width = 279`) and `1024×768` (`extra = 224`, `subpanel_width = 503`) layouts bit-for-bit while keeping `GAUGE` anchored at `x = 413` for every wider viewport (`1280`, `1440`, `1600`, `1920`, etc.).
- **Relationships panel scaling (`crates/sims-ui/src/hud.rs::relationships`):**
  - `Sims.exe` (`0x519247..0x51926a`) displays `5` relationships per page at `800 px` and `10` per page at `1024 px`.
  - Computing `per_page = ((panel.width - arrow_w - 2 - left) / 45).max(1)` yields `5` at `800 px` (bit-exact), `10` at `1024 px` (matching native 1.91), `15` at `1280 px`, and `22` at `1600 px`, while clamping the row right edge to `panel.width - arrow_w - 2` so entries never collide with `PAGE_ON`.
- **Right-bevel preservation on `PanelBack.bmp` (`crates/sims-ui/src/hud.rs::draw_with_thumbnails`, `crates/sims-ui/src/camera.rs::draw_strip_bkg`):**
  - `PanelBack.bmp` (`id::STRIP`, `4910`, `804×100` 8bpp RLE) has uniform navy `(0, 0, 82)` across its interior and an 8-pixel dark right bezel on columns `796..804` (`(0, 0, 49)`).
  - For `r.width <= w` (`<= 1024 px`), rendering takes the unchanged direct path (100% bit-exact at `800×600` and `1024×768`). For `r.width > w` (`> 1024 px`), columns `0..(w - 8)` draw on the left, interior column `w - 16` tiles across the extra width, and the 8-pixel bezel `(w - 8)..w` draws flush against the right edge.

### 2.2 Gray Wedge Next to the Camera Button (`kBackPatch`, `2036`)

- **Why the gray wedge appeared over grass but not over some objects:**
  1. Whenever the bottom strip is expanded, `Game::house_viewport_size()` (`crates/sims-game/src/game.rs`) shortens the 3D house viewport height to `height - 100` (`height - STRIP`), matching `Sims.exe` (`0x4f4b43`).
  2. The renderer clears the full canvas to day lot void `sims_render::scene::BACKGROUND` (`RGB(115, 113, 115)`) and clips terrain/grass drawing to `(width, height - 100)`, whereas tall object sprites anchored above `height - 100` can still draw pixels below `height - 100`.
  3. `hud::layout` drew `PanelBack.bmp` starting at `x = 220` (`CLUSTER[0]`) and `UniversalBack.TGA` (`2037`, `220×183`) at `x = 0..220`. Because `UniversalBack.TGA` curves inward beside the Camera (`175, 95`) and Options (`193, 125`) buttons, its alpha channel is transparent in the wedge `x = 168..220, y = height - 100..height` — exposing the cleared `RGB(115, 113, 115)` void directly behind the curve!
- **Exact native `Sims.exe` 1.91 mechanism:**
  - `0x50a19b` loads resource `0x7f4` = **`2036`** (`cpanel\Backgrounds\BackPatch.bmp`, `52×100`, 24-bit opaque RGB BMP) into `[esi + 0x100]`.
  - `0x50a5e0..0x50a619` draws `BackPatch.bmp` at `0x6f3630 = (168, 50)` inside the `height - 150` shared panel container = **`(168, height - 100)`**, **behind** `UniversalBack.TGA`, whenever the strip is expanded (`168 + 52 = 220`, with a smooth lavender-to-navy gradient on columns `48..51` that joins column `0` of `PanelBack.bmp` at `x = 220`).
  - Verified against original 1.91 capture `live1.png`: pixels in `x = 168..220, y = 500..600` match `BackPatch.bmp` behind `UniversalBack.TGA`.
  - Likewise, opening Camera Mode (`0x4d1712..0x4d174e`, `0x53a543`) expands the strip first so `BackPatch.bmp` fills the transparent corner behind `UniversalBack.TGA` + `CameraPatch.bmp` (`2035`).
- **Fix:**
  - Added `hud::id::BACK_PATCH = 2036` to `hud::RESOURCES` (and a matching `52×100` clean-room procedural bitmap to `Art::clean_room_procedural()`).
  - Emitted `Kind::Image(id::BACK_PATCH)` at `Rect::new(168, height - STRIP, 52, STRIP)` before `id::CLUSTER` whenever `controls.expanded` is `true` (covering Live, Buy, Build, Options, and Camera modes, while leaving collapsed-strip frames — including all 8 `ratchet.txt` scenes — 100.00% bit-exact unchanged).

### 2.3 Authentic `Sims.exe` 1.91 Hotkeys & Text-Input Guard

Disassembled directly from `Sims.exe` 1.91 (`0x4d0ef0` `OnChar`, `0x4d12b1` `OnKeyDown`, `0x51ba1c` `cWinLive::OnKeyDown`, and `UIText.iff` `STR# 161`):

| Key | Native Reference | Action |
|---|---|---|
| `` ` `` / `~` / `KeyCode::Backquote` / `P` | `0x4d0f37..0x4d0fb8` → `0x53b92b` | Toggle Pause (non-repeat) |
| `1` / `2` / `3` | `0x4d0f37..0x4d0fb8` | Normal / Fast / Ultra speed (unpauses if paused) |
| `F1` | `0x4d1826` | Live Mode (or close Camera strip / toggle panel collapse if already in Live Mode) |
| `F2` | `0x4d1787` | Buy Mode (or toggle `ByFunction` ↔ `ByRoom` sort if already in Buy Mode) |
| `F3` | `0x4d1750` | Build Mode |
| `F4` | `0x4d1712` | Toggle Camera / Photo Mode strip (expands strip if collapsed) |
| `F5` | `0x4d16e2` | Options Mode |
| `F10` | Player-expected panel toggle (in Lot mode; Neighborhood keeps Photo Album) | Toggle control panel collapse/expand (`Sims.exe` `0x4d166b` took a `.jpg` snapshot, while `F1` collapsed Live mode; binding `F10` to `toggle_panel` in Lot mode provides the standard quick hide/show control panel hotkey) |
| `Space` | `0x51ba2d` (`cWinLive::OnKeyDown`) | Select next Sim in household (wraps around, closes pie menu) |
| `Enter` | `0x4d1330` → `0x4d14d2` | Center camera on currently selected Sim |
| `Tab` | `0x4d1324` → `0x4d150c` | Toggle floor 1 ↔ 2 |
| `PageUp` / `PageDown` | `0x4d1341` / `0x4d134a` | Floor up / Floor down |
| `Home` / `End` | `0x4d135b` / `0x4d1393` | Step walls Up (`Down → Cutaway → Up`) / Down (`Up → Cutaway → Down`), `UI_error` at bounds |
| `,` / `.` (plus `<` / `>`, `[` / `]`) | `0x4d0fde..0x4d100f` | Rotate held Buy object if holding one, else rotate camera |
| `+` / `=` / `NumpadAdd`, `-` / `_` / `NumpadSubtract` | `0x4d0f35..0x4d0f86` | Zoom in / Zoom out |
| `Arrow` keys | `0x4d12b1` | Pan camera |
| `C` | `STR# 161` | Take snapshot / photo |
| `Z` / `X` | `0x4d11b5` / `0x4d1224` | Previous / Next catalog page (both Buy and Build modes) |
| `Delete` / `Backspace` | `0x486b5b` | Sell held object in Buy Mode |
| `Ctrl/Cmd+S` | `STR# 161[30]` | Save lot (relocated from `F5`) |
| `Ctrl/Cmd+Z` / `Ctrl/Cmd+Y` / `Ctrl/Cmd+Shift+Z` | `0x4cf2e0` / `0x4cf317` | Undo / Redo (Buy & Build) |
| `Ctrl/Cmd+Shift+C` | Cheat console | Toggle Cheat Console |

**Strict text-input guard (`LotInputFocus` / `lot_hotkeys_blocked`):** All bare Lot hotkeys (and `F6`/`Ctrl+M` co-op toggle while Cheat Console is open) are suppressed whenever any text-input or modal surface is active: Cheat Console, Text Entry / Modal Dialogs, Photo Album caption editor, Co-op Lobby, or Create-a-Family.

---

## 3. Verification

- **Bit-exact preservation at `800×600` & `1024×768`:**
  - `sims-game --test ratchet`: `8/8 ratchet: ok` (all 8 collapsed-strip baseline scenes 100.00% bit-exact).
  - `sims-ui --test ui_ratchet`: `4/4` passed.
  - `sims-ui --test hud`: `"closed 800x600"` digest unchanged; expanded-strip digests re-pinned with `BackPatch.bmp` (and `1280×720` with anchored `x = 413` gadget).
  - Goth 43,200-tick determinism hash (`df6eb544...`) unchanged.
- **Visual verification:** Rendered and inspected screenshots at `800×600`, `1024×768`, `1280×720`, and `1600×900` across Live Mode (Needs, Relationships, Personality, Job, House), Buy, Build, Options, and Camera Mode — confirming fixed `x = 413` gadget anchoring, seamless `PanelBack.bmp` extension without dark vertical bands, and `0` leaked gray background pixels in `x = 168..220, y = height - 100..height`.
- **Unit & headless integration tests:** Added layout, `BackPatch` zero-leak, relationship per-page scaling, right-bezel stretching, complete `classify_lot_hotkey` + `lot_hotkeys_blocked` unit tests, and headless Goth house hotkey/compositing tests (`crates/sims-game/tests/native_hotkeys.rs`).
