# Control-panel money visibility, non-residential family friends, pause blink, help button, and pet panel refresh

*Decided 2026-11-21.*

Ports five disassembly-verified behaviors of the Complete Collection 1.91
live-mode control panel across `crates/sims-ui`, `crates/sims-game`, and
`crates/sims-viewer`: budget-mode hiding of funds and family friends,
non-residential selected-Sim family-friends lookup, 4x2 pause-button
rendering and its 500 ms blink timer, interactive hover/click on `kHelpBtn`,
and pet selection panel refresh / gadget button disabling.

## Findings from the 1.91 binary

- **Money & family-friends visibility (`0x4f24ec..0x4f2503`, `0x4cfe0a`,
  `0x4cfe18`, `0x53b2a4..0x53b2e1`):** At `0x4f24fa`, the lot compares
  global 32 (`word ptr [sim + 0x50]`, budget mode) against `1`
  (`cmp word ptr [eax + 0x50], 1; sete al`) and stores the boolean via
  `0x4cfe0a` (`byte ptr [model + 4]`). The control-panel refresh reads it
  via `0x4cfe18` at `0x53b2a6`: when equal to `1`, it hides (`vtable +0x98`)
  both the money control (`[esi + 0x108]`) and the family-friends control
  (`[esi + 0x1d0]`); otherwise it shows (`vtable +0x94`) both.
  `Snapshot::show_money` tracks `globals[32] != 1` and `hud::layout` omits
  both items when false.
- **Non-residential family-friends lookup (`0x4cfcdb..0x4cfd7f`,
  `0x425d69`):** `0x4cfcdb` first checks `0x40745f` (whether the lot is
  residential). On residential lots (`0x4cfd4a`) it reads the lot's family
  (global 9) and calls `0x410260` -> `0x427219` (recomputing if stale). On
  non-residential lots (`0x4cfd1a..0x4cfd48`) it queries the selected Sim
  (`0x4e986d`, `0x4e9608`), reads `word ptr [person + 0x606]` (person data
  word 61, `sims_sim::neighbors::FAMILY`), looks up that family linearly in
  the neighborhood's family vector via `0x425d69`, and calls `0x410260` (or
  returns `0` when nobody is selected or no matching family exists).
  `Snapshot::with_zoning` and `Snapshot::with_relations` now follow this
  exact branch using `Lot::residential()`.
- **Pause button 4x2 cell selection & 500 ms blink (`0x538cd8`,
  `0x53b4b0..0x53b7e7`, `0x5e3b40..0x5e3c41`, `0x5e4b0f`):** `id::PAUSE`
  (`0x7ea`, `cpanel\Buttons\pause.bmp`, 60x30) is a 4-column by 2-row
  `cGZWinBtn` sprite sheet (15x15 per cell). `0x5e3b40..0x5e3c3e` selects
  column 3 when disabled, 1 when selected (even while hovered), 2 when
  hovered, and 0 when normal; `0x5e4b0f` draws at `(col * 15, row * 15)`.
  Row 0 col 1 is cyan selected and row 1 col 1 is red selected (while col 0
  of both rows is the unlit normal icon, so the previous `Rect::new(0, 15, 15, 15)`
  drew normal gray-blue pixels while paused). Entering Live-mode pause
  (`state 2`, `0x53b707..0x53b74d`) sets `selected = 1` while leaving
  `row = 0` (cyan) and arms a 500 ms (`0x1f4`) timer (`0x5daf06`); each
  timer tick (`0x53b500..0x53b52e`) toggles row `0 <-> 1` (cyan <-> red)
  and re-arms 500 ms. Non-Live pause (`state 1`, `0x53b736..0x53b74d`) sets
  steady `selected = 1, row = 1` (red) without a timer, and unpausing
  (`0x53b757..0x53b7e3`) cancels the timer and resets `selected = 0, row = 0`.
  `hud::Kind::Pause { paused, lit }`, `hud::PAUSE_BLINK_US = 500_000`, and
  `sims-game`'s `PauseBlink` timer implement this state machine.
- **Interactive `kHelpBtn` (`0x538f41..0x538fe6`, `0x53aaad..0x53ab5a`):**
  `kHelpBtn` (`4993` at `(5, 164)`, 40x15 four-cell strip) is constructed
  as an enabled `cGZWinBtn` and opens the Help System dialog (`0x50e8d3`)
  on click. Promoted from a static `Kind::Cell` to `Kind::Button` with
  `Command::OpenHelp` (presentation-only), enabling cell-2 hover feedback
  and click dispatch while keeping unhovered pixels identical.
- **Pet selection refresh & gadget disabling (`0x51c5a6..0x51c9e5`,
  `0x51ce13..0x51ce36`):** When a selected household Sim has
  `word ptr [person + 0x60e] > 1` (`person.data[65] > 1`,
  `sims_sim::neighbors::KIND > 1`), the live-panel selection refresh
  disables (`byte ptr [btn + 0x168] = 0`) both the Interests
  (`[esi + 0x170]`, `0x51c982`) and Gifts (`[esi + 0x174]`, `0x51c9da`)
  buttons, and if either panel (`6` or `7`) is currently open, switches to
  Needs (`0x51ce1d..0x51ce31 -> 0x51d375(1)`), closing any open help
  balloon (`0x50fb7f`, `UI_whoosh`). `Resident::pet`, `hud::Selected`,
  `Controls::follow`, `hud::check_press`, and `live_strip` now enforce
  both rules.

## Ratchet impact

Fixing `Kind::Pause` to draw column 1 (`Cell::Selected`, cyan row 0 on
initial Live-mode pause) instead of column 0 (normal gray-blue) removes
all 72 pause-button pixel differences in the two scenes whose pause button
is unmasked (`near-r2-0712-bench` and `near-r2-0805`), with zero
regressions in any class on any scene:

| Scene | `frame` before | `frame` after | `panel` before | `panel` after |
| --- | ---: | ---: | ---: | ---: |
| `near-r2-0712-bench` | 99.00% | 99.02% | 99.76% | 99.93% |
| `near-r2-0805` | 98.37% | 98.38% | 99.82% | 99.99% |
| `far-r0-cut-0816` | 99.52% | 99.52% | 99.88% | 99.88% |
| `far-r1-cut-0816` | 99.22% | 99.22% | 98.97% | 98.97% |
| `medium-r1-cut-0816` | 99.48% | 99.48% | 99.87% | 99.87% |
| `near-r3-down-0816` | 98.32% | 98.32% | 99.88% | 99.88% |
| `near-r0-up-0816` | 99.51% | 99.51% | 99.88% | 99.88% |
| `night-far-r0-cut-2153` | 99.54% | 99.54% | 99.19% | 99.19% |

The six scenes with `leave_out = 86 554 98 565` keep identical class
shares and update only their whole-frame `ours` SHA-256 digests in
`crates/sims-game/tests/ratchet.txt`.

## Not ported / what would reopen this

- The Help System dialog window itself (`0x50e8d3`, `0x53aaad..0x53ab5a`)
  and modal-dialog steady-red pause transition (`state 1`, `0x4d3d98`).
- Ultra-speed blinking (`state 3`, `0x53b4c9..0x53b4fe`) during idle speed
  when the saved return speed is not Ultra.
- Pet motive panel swapping (`[esi + 0x100]` vs `[esi + 0x104]` at
  `0x51ca0a..0x51ca4e`).
