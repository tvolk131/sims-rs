# Options panel (`cWinOptions`) and native dialog box chrome (`cTSWinMsgBox` / `cTSWinTextEdit2`)

Date: 2026-11-21

## Summary

Implemented the Complete Collection 1.91 Options mode panel (`cWinOptions`) with
its three subpanels (Graphics, Sound, Play), Options help balloons (`STR# 145` +
`kPopupOpt*` `3700..=3717`), Quick Tips hover tooltips, Options mode simulation
and audio pausing, real control and action wiring (Save, Quit, Free Will global
30, Shadows/Lighting renderer toggles, Edge Scrolling, Sound volume sliders
wired into `sims-audio::Player` kinds 37–39, Sim In Background), and
disassembly-exact interactive dialog chrome (`PopupInfoTiles.bmp` 3008 and
`GenDlg.bmp` 22 9-slice frames, `SimStub.BMP` 4992 portrait backing,
`cTSWinTextEdit2` initial selection and end-to-end VM kind 3 + window keyboard
input, and mouse-up `cTSWinBtn` button activation).

## Disassembly Evidence (Complete Collection 1.91)

### Options Panel (`cWinOptions`, vtable `0x66c428`)

- **Container placement:** `0x50992d..0x509941` (`[0x6f3640] = 220`,
  `[0x6f3644] = 50`) and `0x509f1c..0x509f81` place `cWinOptions` at
  `(220, height - 100)` with width `width - 220` and height `100`.
- **Mode entry, reopen, and simulation/audio pause:**
  - `0x53a51b..0x53a53e → 0x4cea68..0x4cea73`: clicking the Options mode button
    always passes `toggle = 0` (`SetMode(Mode::Options)`), expanding when
    collapsed and keeping an expanded Options strip open.
  - `0x5179eb..0x5179f6`: `cWinOptions::Show` calls `ShowSubpanel(0)`, resetting
    the active tab to `None` whenever Options becomes visible.
  - `0x4ce567` / `0x46d566..0x46d633` / `0x4ce4c4..0x4ce4d1` /
    `0x517cf9 → 0x4d3d98 → 0x4d1a9f`: entering Options or opening a UI
    confirmation dialog suspends lot ticks (`Game::advance` returns 0), and
    non-Live modes pause live audio via `SoundEvent::Pause` / `Unpause`.
  - `0x4f762b..0x4f763c`, `0x4f76d5`: window focus loss/regain only saves and
    restores the pause state in Live mode (`mode == 2`) when `Sim In Background`
    (`!byte [0x6c9fa1]`) is off; `Input::Active(focused)` is dispatched
    independently in all modes.
- **Artwork (`cWinOptions::Init`, `0x513ef0..0x513f5a`, `0x66c364..0x66c378`, `0x51599d`):**
  - Dividers: `kOptToothPick1 = 4916` (`OptionsToothpkHoriz.bmp`, 201×2) at
    `(16, 51)` (`0x516160`) and `kOptToothPick2 = 4917` (`OptionsToothpkVert.bmp`,
    2×89) at `(222, 6)` (`0x5161b6`).
  - Action buttons: `kOptionNbhd = 4951`, `kOptionSave = 4950`, `kOptionExit = 4952`.
  - Tab buttons: `kOptionGraphics = 4957`, `kOptionSound = 4958`, `kOptionPlay = 4959`.
  - Controls: `kOptCheckbox = 4953` (6 cells of 20×21), `kOptRadio = 4954`
    (6 cells of 20×20), `kOptSliderOn = 4955` / `kOptSliderOff = 4956`
    (4 cells of 11×14), `kOptionTutorialReset = 4960` (4 cells of 22×30 at
    `(strip.width - 25, 7)`).
  - **6-cell toggle state selection (`cTSWinBtn::GetCell`, `0x5e3ba1..0x5e3c3e`):**
    For 6-cell strips (`edx == 6`), `0x5e3ba1` executes `push 4; pop ebx`
    (setting `ebx = 4`) before jumping to `0x5e3be6`. When disabled (`edi == 0`),
    `0x5e3be8 → 0x5e3c13 → 0x5e3c15 → 0x5e3c23`: unchecked (`esi == 0`) jumps
    to `0x5e3b96` (`mov [ecx + 0xe8], ebx`), selecting cell **`4`** (disabled
    unchecked), and checked (`esi != 0`) stores **`5`** (`0x5e3c2b`).
- **Layout strings, Play right column Y offset, help balloons & Quick Tips (`UIText.iff STR# 145` & `153`):**
  - `0x5140db` loads `STR# 145` and parses `"(x;y)"` pairs via `0x5001cc`.
  - Graphics (`0x514428..0x514f20`): Anti-alias occupies row 0 and is hidden
    after layout (`0x517485..0x51749b`), leaving Shadows, Lighting, and
    Interface Effects on rows 1–3 alongside Terrain Detail and Character Detail.
  - Play right column Y (`0x51584c..0x515852`, `0x515b44..0x515b4a`,
    `0x515e14..0x515e17`): the running Y mutates by `cb_y_offset = (label_h - 21) / 2`
    then adds `+4` in English (`[ebp - 0x25] == 0`), placing right-column labels
    at `play_origin[1] + cb_y_offset + 4` (+2 px below the left column with the
    installed 17 px 9 pt font) and checkboxes at `right_label_y + cb_y_offset`.
  - Setting help balloons (`0x516719..0x51679a`, `0x66c394..0x66c410`,
    `0x517957..0x5179cd`): clicking any setting label toggles its help balloon
    with title/body from `STR# 145` and its `kPopupOpt*` bitmap (`3700..=3717`),
    playing `UI_whoosh`, and moving the pointer outside the Options panel or
    leaving the window dismisses it with `UI_whoosh`.
  - Escape in Options (`0x517904..0x517954`, `0x5175ab..0x5175bb → 0x50fb7f`):
    closes any active Options subpanel tab and open help balloon (playing
    `UI_whoosh` + `UI_click`) and consumes Escape rather than exiting the game.
  - Narrow-window reflow (`docs/vision.md`): at `>= 800` px wide, all three
    Options subpanels use exact 1.91 coordinates and help balloons right-anchor
    at `window_w - width` (`0x5100b2..0x5100c7`) attached above the cluster
    (`0x514008..0x51401b`); below `800` px wide (including `640×480`, `600×480`,
    and `500×480`), subpanel columns and (below `580` px) action/tab buttons
    compact into disjoint bands with fitted labels and slider widths, and help
    balloons clamp and wrap inside the window above the cluster so no control,
    header, label, or help line clips or overlaps.
  - Quick Tips (`0x5164ef..0x5164ff → 0x517a6f`): enables/disables hover
    tooltips on the action, tab, and Reset Tutorial buttons using `STR# 145`
    entries `[1, 3, 5, 7, 9, 11, 69]`.
- **Per-lot visibility (`0x4f2490..0x4f24e7`, `0x5174ac..0x5174c5`, `0x5177aa..0x517833`):**
  - `no_save` hides Save on zoning-2 and Magic Town lots `93..=99` while
    visiting (`!edit_mode`), and on unoccupied non-zoning-2 lots while visiting.
  - Play hides both the Free Will label and checkbox on Downtown (`21..=30`),
    Vacation (`81..=89`), and Magic Town (`93..=98`), while showing Free Will on
    residential, Community (`40..=49`), and Studio Town lots. Reset Tutorial
    shows only on residential lots (`zoning == 1`).
- **Action confirmations & dirty-lot gating (`0x516809..0x516883`, `0x5170f0..0x51713c`, `0x517c03 → 0x50d68a`, `0x4f290d`, `0x4f1a11`, `0x4f25b4`):**
  - Options action buttons (`Save`, `Neighborhood`, `Quit`, `Reset Tutorial`)
    capture on mouse-down, render their pressed cell while held inside the
    button (`0x5e44dd..0x5e44e1`), and hit-test the live layout rectangle on
    mouse-up (`0x514170..0x514176`, `0x5e45d2..0x5e45f3`) so dragging off or
    resizing mid-press cancels cleanly (modal dialog buttons also relayout on
    resize).
  - Edit-mode Save Card (`cWinEditLotInfo`, `0x517c03 → 0x50d68a..0x50e42c`) is
    gated on `[lot + 0x50] == 1` edit mode (`0x517c21`, reached organically when
    booting any vacant zoned lot such as residential `House02.iff` or
    Downtown/Vacation/Community/Studio/Magic lots with `family == 0` per
    `0x4f1b5f..0x4f1b76`, with empty `game + 0x10` active family during
    boot/unoccupied visitor passes at `0x42a2b4..0x42a2e5` and `0x445501`,
    setting global 32 = 1 for the §999,999 edit budget at `0x4f1bd9..0x4f1be5`
    / `0x46d8c9`, keeping `edit_mode` synced with global 32 across `restore()`,
    disabling the Live mode button (`0x4f2482..0x4f248b → 0x53b233..0x53b254`),
    and opening in Buy mode per `0x4f2510..0x4f2520`), uses frame `3008`
    (`PopupInfoTiles.bmp`, `0x50d8a1`) around `SAVE_CARD` (`4994`, `0x50d8b4`)
    with exact `0x50d9a2..0x50d9e5` height and `0x50da8f..0x50dc16` field bounds
    (`card_w - 8` = 329 px wide, Lot Description `card_h - 68` = 96 px tall),
    loads initial lot strings from `*Desc.iff STR# (2000 + lot_id)`
    (`0x50e14b..0x50e253`: `21..=30` `DTDesc.iff`, `40..=48` `VIDesc.iff`,
    `81..=89` `STDesc.iff`, `90..=99` `MTDesc.iff`, else `NeighborhoodDesc.iff`
    only when `0x42b9e3` zoning is nonzero), exposes editable Lot Name (49 chars,
    `0x31`) and multiline Lot Description (254 chars, `0xfe`, Return inserts
    `'\n'` at `0x5f28b6..0x5f28f9`, row-aware click hit-testing
    `0x5f2475..0x5f2522`, captured mouse-drag text selection
    `0x5f27a6..0x5f2887`, visual-row Home/End `0x5f29f2..0x5f2c13`, and
    character-column Up/Down `0x5f37f4..0x5f38cd`), renders the centered
    `STR# 153[5]` prompt on 3-button cards (`0x50dcc8..0x50e056`), marks the
    lot dirty on metadata changes before the host save attempt, and persists
    accepted edits in `sims_vm::boot::save::Saved` format `6`.
  - Free Will (`0x4e66eb..0x4e6704 → 0x432220`) is applied from the persisted
    Options preference to lot global 30 on both `Game::open` and `Game::restore`
    (including answered-awaiting-replay dialog snapshots), Edge Scrolling tests
    the 32-pixel windowed scroll band against the house viewport bottom in all
    modes (`0x4d25b6..0x4d25c7 → 0x4d4586..0x4d45c8 → 0x4d4843`) at the native
    `562.5` px/s straight-edge rate (`0x4d4440`, `0x4d4839`, `0x4d4946`,
    `0x4d499d`, `0x4d4a06`) and `562.5` px/s vector-capped corner rate
    (`0x4d4893`, `0x4d48f9`, `0x4d4ab0..0x4d4ad8`), with `dt` capped at 100 ms
    (`0x4d50ef..0x4d5160`) and an exact fixed-point accumulator
    (`1 / 16_000_000_000` px) for split-invariant displacement, bypassed during
    active right-drag panning (`0x4d46bd..0x4d4704`) and never short-circuited
    by animation redraws (`0x4d4af8..0x4d4b06`), and Make New
    Character names (`0x44b0a8..0x44b0fc`, `0x44b18a..0x44b1cc`) sync into the
    HUD `Faces` cache on steps/draws and reset to boot + snapshot names on
    `Game::restore`.
  - Save (`0x516874`) proceeds directly on ordinary lots (`Outcome::save = true`)
    and opens the 1-button Save Card in edit mode.
  - Quit (`0x516809..0x51686f → 0x4f290d`):
    - clean/non-savable lots open the 2-button Yes/No `STR# 153[1..2]`
      `cTSWinGenDlg` confirmation;
    - dirty ordinary lots open the 3-button Yes/No/Cancel `STR# 153[5..6]`
      `cTSWinGenDlg` confirmation (Save & Quit exits only if saving succeeds,
      `0x4f29cc`);
    - dirty edit-mode lots open the 3-button Save Card: **Yes** saves
      (`0x4f26cf`) and enters the follow-up `ConfirmQuit` (`0x516826 → 0x516866`,
      upgrading to `ConfirmSaveBeforeQuit` if saving failed), **No** temporarily
      clears `dirty` and opens `ConfirmQuit` while restoring `dirty` if the
      player declines (`0x516828..0x516852`), and **Cancel** returns immediately.
  - Neighborhood (`0x5170f0 → 0x4f1a11`) prompts only when `dirty && can_save`
    (3-button `cTSWinGenDlg` `STR# 153[3..4]` on ordinary lots, 3-button Save
    Card in edit mode).
  - Reset Tutorial (`0x4f25b4..0x4f25f5`) opens the 2-button `cTSWinGenDlg`
    `STR# 153[7..8]` confirmation and, on Yes with a dirty savable lot, chains
    through the native `0x4f1a11` save confirmation (3-button `STR# 153[3..4]`
    or edit-mode Save Card) before reporting the unported tutorial overwrite.
- **Preferences persistence (`UserData/Options.iff` `Optn` 128, `0x4e6505..0x4e685e`):**
  - `OptionsState::load_install` decodes the exact 36-byte `Optn` 128 layout
    (`[14]` `[esi+0]`, `[15]` `anti_alias`, `[16]` `shadows`, `[17]` `lighting`,
    `[18..19]` detail, `[20..22]` volumes `[8, 8, 8]`, `[23..26]` play left,
    `[27]` `interface_effects`, `[28..32]` `[esi+8]`, `[32..35]` play right,
    defaulting missing v1–v3 flags to enabled per `0x4e650f..0x4e6513`,
    `0x4e6814..0x4e681a`, and `0x4e6842`),
    applies persisted Free Will to lot global 30 on startup (`0x4e6704 → 0x432220`),
    and persists user changes in `Optn` 128 IFF format beside `--saves`
    (`*.options.iff`) across relaunches.
- **Control wiring:**
  - Sound sliders (`0x5f5d73..0x5f6500`, `0x516624..0x5166b2`): `0..=10` steps
    wired to `sims_audio::Player::set_*_volume_step` (kinds 37–39). Pointer
    capture (`0x5f6245..0x5f625b`) survives window exit and evaluates the live
    out-of-window release x-coordinate (`0x5f62ba..0x5f62de`, querying Quartz
    `CGEvent::location` on macOS where winit filters outside `mouse_motion`
    after button release).
  - Free Will (`0x432220..0x432242 → 0x46d9c4(0x1e, al)`): reads/writes lot
    global 30 (`world.globals[30]`).
  - Shadows (`0x4e5fb2`, `[0x6f0188]`, `0x4ae846`) & Lighting (`0x4e5fd5`,
    `[0x6f018a]`, `0x4c7c1a` wall lamp pools, `0x4c835c` floor lamp pools,
    object shading precedence at `0x437abd..0x437aed` preserving the `lit_at_brightest`
    time-of-day cap over `glows`): wired to `sims_render::RenderOptions` while
    preserving base room shading.
  - Edge Scrolling (`0x4e6246..0x4e6286`): wired to `Game::edge_scroll` in the
    window tick loop.
  - Terrain/Character Detail LOD, Interface Effects translucency, Auto-Centering
    event jumps, Auto Snapshot, Live PIP, and Export HTML are persisted in
    `OptionsState` (`Optn` 128); their underlying engine subsystems are not yet
    ported.

### Native Interactive Dialog Chrome (`cTSWinMsgBox`, `cTSWinGenDlg`, `cWinEditLotInfo`, `cTSWinTextEdit2`, `cTSWinBtn`)

- **9-slice frames (`0x4efe3b..0x4efe59`, `0x523528`, `0x524603..0x524645`, `0x4efec9..0x4efede`, `0x5e8f6a`):**
  - `FrameStyle::Popup` renders `PopupInfoTiles.bmp` (`3008`, 36×36, 12×12 tiles)
    for VM `cTSWinMsgBox` dialogs and the edit-mode Save Card (`0x50d8a1`),
    matching all 19,656 perimeter pixels of the captured Magic Town frame after
    RGB565 normalization (updating the digest pin in `crates/sims-ui/tests/dialog.rs`
    to `f2a4498bc09ed14faf3b7d198d5e8fd57dec18f52d1545ae8109f27c91885eaf`).
  - `FrameStyle::Generic` implements `cTSWinGenDlg` geometry
    (`0x5e542f..0x5e589b`, `0x5e5d46..0x5e5eac`: upper-left 12 pt title at
    `(16, 16)`, centered body below, `396×135` for the 3-button Neighborhood
    Save prompt) and renders slot 5 (`0x5e5441` → `PopupInfoTiles.bmp` `3008`),
    matching all 10,220 perimeter pixels of the 1.91 capture after RGB565
    normalization.
- **45×45 portrait backing (`0x523572`, `0x524660..0x524782`, `0x524a6c..0x524ae7`):**
  45×45 dialog icons reserve `SimStub.BMP` (`4992`, 133×103) and center over it.
- **`cTSWinTextEdit2`, VM kind 3, and `Make New Character` (`0x436a52..0x436c76`, `0x44b0a8..0x44b1cc`, `0x451a6b..0x451a85`, `0x523820..0x5238f3`, `0x5f2000..0x5f2388`, `0x5f2c13`, `0x5f462f..0x5f4685`):**
  - `sims-vm` accepts Dialog kind 3 (`answers(3) == Some(&[FIRST])`), marks
    `Shown::text_entry = true` so `Answers::choices` waits for player text input
    rather than auto-accepting, writes submitted `Answer::text` immediately into
    the live `dialog_text` slot (`[0x6e4ac0]` at `0x451a77`) as well as
    `Update::dialog_text` / `Lot::last_dialog_text`, consumes that text in
    `Make New Character` (`0x44b0a8..0x44b1cc`, falling back to localized
    `UIText.iff STR# 142[5]` on empty submissions) to name newly created
    children and pets (passed into `Lot::advance`, `Lot::pie_menu`,
    `Lot::highlight`, `Lot::pick`, `Lot::queue_interaction`, and `Lot::cancel`
    and persisted in `sims_vm::boot::save` format `6`), and takes the true
    branch.
  - Nonempty initial entry text opens fully selected (`selection = Some((0, len))`,
    highlighted in `draw_entry` at `0x5f2107..0x5f2125`) so the first typed
    character replaces it (`0x5f462f..0x5f4685`); `Home` and `End` clear the
    selection unconditionally (`0x5f2c13`).
- **Modal input blocking, confirmation keys, & mouse-up `WinBtn.bmp` (`0x4ff509..0x4ff538`, `0x4f370d..0x4f374e`, `0x5e3b40..0x5e3c3e`, `0x5e44b4..0x5e45fc`, `0x5e5b3b..0x5e5bfd`, `0x6d7530`):**
  - Opening any UI or VM modal dialog lays it out immediately and blocks pointer
    presses, mouse-wheel zoom, right-drag panning, and global shortcuts before
    the next frame draws.
  - Confirmation dialogs handle `Enter` (button 0), `Escape` (cancel/last
    button), and localized `UIText.iff STR# 161[7..8]` Yes/No hotkeys
    (`[0x6d7530]`); text-entry dialogs route editing keys, `Tab` (Save Card
    field switch), `Enter` (inserting `'\n'` in multiline Lot Description,
    submitting otherwise), and `Escape` (multi-button Cancel).
  - Mouse-down captures and sets `pressed_button` (rendering cell 1 with cyan
    text only while the pointer remains inside the button); mouse-up releases
    capture and activates only if the pointer is still inside that same enabled
    button.
