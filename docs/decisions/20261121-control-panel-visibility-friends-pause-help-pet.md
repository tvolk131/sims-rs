# Control-panel money visibility, non-residential family friends, pause blink, help button, and pet panel refresh

*Decided 2026-11-21.*

Ports five disassembly-verified behaviors of the Complete Collection 1.91
live-mode control panel across `crates/sims-vm`, `crates/sims-ui`,
`crates/sims-game`, and `crates/sims-viewer`: budget-mode hiding of funds
and family friends, non-residential selected-Sim family-friends lookup and
cache refresh, 4x2 pause-button rendering and its 500 ms blink timer,
interactive hover/click on `kHelpBtn`, and pet/selection panel refresh and
gadget button disabling.

## Findings from the 1.91 binary

- **Money & family-friends visibility (`0x4f24ec..0x4f2503`, `0x4cfe0a`,
  `0x4cfe18`, `0x53b2a4..0x53b2e1`):** At `0x4f24fa`, the lot compares
  global 32 (`word ptr [sim + 0x50]`, budget mode) against `1`
  (`cmp word ptr [eax + 0x50], 1; sete al`) and stores the boolean via
  `0x4cfe0a` (`byte ptr [model + 4]`) during lot load. The control-panel
  refresh reads it via `0x4cfe18` at `0x53b2a6`: when equal to `1`, it
  hides (`vtable +0x98`) both the money control (`[esi + 0x108]`) and the
  family-friends control (`[esi + 0x1d0]`); otherwise it shows
  (`vtable +0x94`) both. `Snapshot::show_money` tracks `globals[32] != 1`
  and `hud::layout` omits both items when false.
- **Non-residential family-friends lookup & cache refresh (`0x4cfcdb..0x4cfd7f`,
  `0x425d69`, `0x4ea8ed..0x4ea8f7`, `0x531095..0x5310a4`):** The cluster
  counter (`0x4cfcdb`) first checks `0x40745f` (whether the lot is
  residential: global 10 is 0 or has neighborhood zoning 1). On
  residential lots (`0x4cfd4a`) it reads the lot's family (global 9) and
  calls `0x410260` -> `0x427219` (clearing `stale` and caching the count
  at `0x427ca0..0x427ca7`). On non-residential lots (`0x4cfd1a..0x4cfd48`)
  it queries the selected Sim (`0x4e986d`, `0x4e9608`), reads
  `word ptr [person + 0x606]` (person data word 61,
  `sims_sim::neighbors::FAMILY`), looks up that family linearly via
  `0x425d69`, and calls `0x410260` (or returns `0` when nobody is selected
  or no matching family exists). By contrast, the Job panel
  (`0x4ea8ed..0x4ea8f7`, `0x531095..0x5310a4`) always reads the active lot
  family (`[0x6e48bc][0] + 0x10`, global 9). Accordingly:
  - `Interpreter::refresh_family_panel` (`crates/sims-vm/src/selection.rs`)
    updates the selected Sim's family cache on non-residential lots and
    global 9's family cache on residential lots, and `Lot::select` calls it
    after changing selection so the simulation cache stays current on
    non-residential lots as well.
  - `Snapshot` keeps `family_friends` (global 9, used by `Kind::Work`)
    separate from `cluster_friends` (`0x4cfcdb`, used by `Kind::Friends`).
  - Both `sims-game` (`Lot::residential()`) and `sims-viewer` (from
    `Neighborhood::zoning`) pass the lot's real residential status into
    `Snapshot::with_zoning` and `hud::check_press`, and read the live
    household roster from `world.families` (`0x4e97dc`) when families are
    loaded.
- **Pause button 4x2 cell selection & 500 ms blink (`0x538cd8`,
  `0x53b4b0..0x53b7e7`, `0x5e3b40..0x5e3c41`, `0x5e4b0f`,
  `0x5daf1c..0x5daf31`):** `id::PAUSE` (`0x7ea`, `cpanel\Buttons\pause.bmp`,
  60x30) is a 4-column by 2-row `cGZWinBtn` sprite sheet (15x15 per cell).
  `0x5e3b40..0x5e3c3e` selects column 3 when disabled, 1 when selected
  (even while hovered), 2 when hovered, and 0 when normal; `0x5e4b0f`
  draws at `(col * 15, row * 15)`. Row 0 col 1 is cyan selected and row 1
  col 1 is red selected (while col 0 of both rows is the unlit normal
  icon, so the previous `Rect::new(0, 15, 15, 15)` drew normal gray-blue
  pixels while paused). Pausing (`state 2`, `0x53b58a -> 0x53b5d5 ->
  0x53b5e8 -> 0x53b707..0x53b74d`, in Live mode or outside Live mode) sets
  `selected = 1` while leaving `row = 0` (cyan) and arms a 500 ms (`0x1f4`)
  timer (`0x5daf06`); each timer callback (`0x53b500..0x53b52e`) toggles
  row `0 <-> 1` (cyan <-> red) once and re-arms 500 ms from the current
  clock (`0x5daf1c..0x5daf31`). While unpaused outside Live mode
  (`state 1`, `0x53b55c..0x53b690 -> 0x53b736..0x53b74d`), the button stays
  steady selected on lit row 1 (red) without a timer, and unpausing in Live
  mode (`0x53b757..0x53b7e3`) cancels the timer and resets `selected = 0,
  row = 0`. `hud::Kind::Pause { paused, lit }`, `hud::PAUSE_BLINK_US =
  500_000`, and `sims-game`'s `PauseBlink` timer implement this state
  machine.
- **Interactive `kHelpBtn` (`0x538f41..0x538fe6`, `0x53aaad..0x53ab5a`):**
  `kHelpBtn` (`4993` at `(5, 164)`, 40x15 four-cell strip) is constructed
  as an enabled `cGZWinBtn` and opens the Help System dialog (`0x50e8d3`)
  on click. Promoted from a static `Kind::Cell` to `Kind::Button` with
  `Command::OpenHelp` (presentation-only), enabling cell-2 hover feedback
  and click dispatch while keeping unhovered pixels identical.
- **Selection refresh & pet/non-household gadget disabling
  (`0x51c5a6..0x51c9e5`, `0x51ce13..0x51ce36`, `0x51d06e..0x51d095`,
  `0x4cf9a4`):**
  - When nobody is selected on the lot (`0x4cf9a4 == -1`), `0x51d06e`
    closes any open panel (closing any open help balloon with `UI_whoosh`),
    while `0x51c5d6..0x51c5e1 -> 0x51c775` exits early and preserves the
    previous enabled state of Interests (`[esi + 0x170]`) and Gifts
    (`[esi + 0x174]`).
  - When a non-household Sim is selected (`0x4cf9a4 != -1`, `+0xe8 == -1`),
    `0x51d06e` keeps the open panel (opening Needs if none) while
    `0x51c609..0x51c619 -> 0x51c8c2` disables Interests and Gifts (and
    gadget presses are refused at `0x51bbe6..0x51be75`).
  - When a human household Sim is selected (`person.data[65] <= 1`),
    `0x51c6f1..0x51c8b3` enables Interests and Gifts.
  - When a pet household Sim is selected (`person.data[65] > 1`,
    `0x51c61f`, `0x51ce13`), `0x51c8c2` disables Interests and Gifts, and
    if either panel (`6` or `7`) is currently open, `0x51ce1d..0x51ce31`
    switches to Needs (`0x51d375(1)`), closing any open help balloon
    (`0x50fb7f`, `UI_whoosh`).
  `Resident::pet`, `hud::Selected` (`None`, `NonHousehold`, `Human`,
  `Pet`), `Controls::interests_gifts_enabled`, `Controls::follow`,
  `hud::check_press`, and `live_strip` model these transitions.

## Ratchet impact

Fixing `Kind::Pause` to draw column 1 (`Cell::Selected`, cyan row 0 on
initial Live-mode pause) instead of column 0 (normal gray-blue) removes
all 72 pause-button pixel differences in the two scenes whose pause button
is unmasked (`near-r2-0712-bench` and `near-r2-0805`), with zero
regressions in any class on any scene:

| Scene | `frame` before | `frame` after | `panel` before | `panel` after |
| --- | ---: | ---: | ---: | ---: |
| `near-r2-0712-bench` | 99.24% | 99.26% | 99.76% | 99.93% |
| `near-r2-0805` | 98.84% | 98.85% | 99.82% | 99.99% |
| `far-r0-cut-0816` | 99.52% | 99.52% | 99.88% | 99.88% |
| `far-r1-cut-0816` | 99.23% | 99.23% | 98.97% | 98.97% |
| `medium-r1-cut-0816` | 99.49% | 99.49% | 99.87% | 99.87% |
| `near-r3-down-0816` | 98.32% | 98.32% | 99.88% | 99.88% |
| `near-r0-up-0816` | 99.51% | 99.51% | 99.88% | 99.88% |
| `night-far-r0-cut-2153` | 99.54% | 99.54% | 99.19% | 99.19% |

The six scenes with `leave_out = 86 554 98 565` keep identical class
shares and update only their whole-frame `ours` SHA-256 digests in
`crates/sims-game/tests/ratchet.txt`.

## Not ported / what would reopen this

- The Help System dialog window itself (`0x50e8d3`, `0x53aaad..0x53ab5a`)
  and modal-dialog steady-red pause transition (`state 1`, `0x4d3d98`).
- `Command::SetMode` switching into Buy/Build/Options in `sims-game`
  (so non-Live steady-red pause is currently exercised only through
  `sims-ui`'s `Controls::mode`).
- Mouse-up release cancellation on control-panel buttons
  (`0x5e45e1..0x5e45f3`): all `sims-ui` panel buttons currently dispatch
  on mouse-down.
- Lot-load caching of `global 32 == 1` into `[model + 4]` (`0x4f2503`):
  `Snapshot::read` rereads `globals[32]` on each frame (matching whenever
  global 32 does not change mid-lot).
- Ultra-speed blinking (`state 3`, `0x53b4c9..0x53b4fe`) during idle speed
  when the saved return speed is not Ultra.
- Pet motive panel swapping (`[esi + 0x100]` vs `[esi + 0x104]` at
  `0x51ca0a..0x51ca4e`).
