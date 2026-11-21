# Pet Personality/Skills subpanels, ultra-speed blink, and neighbor-matched selection

*Decided 2026-11-21.*

Ports four disassembly-verified behaviors of the Complete Collection 1.91
live-mode control panel across `crates/sims-ui` and `crates/sims-game`:
pet Personality trait labels and zodiac hiding, Dog and Cat Pet Skills
subpanels on the Job tab, state-3 ultra-speed 500 ms blinking during idle
speed, and neighbor/GUID selection matching (plus non-residential House
panel suppression).

## Findings from the 1.91 binary

- **Pet Needs panel (`[esi + 0x100]` vs `[esi + 0x104]` at `0x519603..0x519b93`,
  `0x51ca0a..0x51ca4e`, `0x51d694..0x51d6d4`):** Both human (`STR# 130`,
  `0x519621`) and pet (`STR# 180`, `0x5197f3`) Needs panels construct the
  same eight motive meters (`0x66c5e0 = [7, 5, 6, 15, 8, 14, 9, 13]`) with
  identical labels (`Hunger`, `Energy`, `Comfort`, `Fun`, `Hygiene`,
  `Social`, `Bladder`, `Room`) and identical western bounding box
  `(0, 21, 200, 105)` (`0x519a34`, `0x519b23`). Only the unported
  need-help balloon descriptions (`STR# 180` odd/even pairs and balloon
  GUID tables `0x6f3950` vs `0x6f39d0`) differ, so Needs rendering is
  unchanged.
- **Pet Personality labels & zodiac hiding (`0x51df31..0x51e0c9`,
  `0x52157e..0x5215c0`, `0x51cd11..0x51ce05`, `0x51d640..0x51d68f`,
  `0x51d1bf..0x51d1f4`):**
  - At init (`0x51df6a`, `0x51df9c`), each personality bar loads both
    human (`Live.iff STR# 131`: `Neat`, `Outgoing`, `Active`, `Playful`,
    `Nice`) and pet (`Live.iff STR# 170`: `Quiet`, `Friendly`, `Playful`,
    `Smart`, `Loyal`) strings and sizes every bar's label box once from
    the widest human label (`0x51e062..0x51e0c9`, `0x52144a`).
  - On update (`0x52157e..0x5215c0`), when `word [person + 0x60e] > 1`
    (`person.data[65] > 1`), the bar swaps its text to the pet label
    without resizing.
  - Both panel switch (`0x51d650..0x51d663`) and selection/personality
    refresh (`0x51cd17..0x51ce05`, `0x51d1cc..0x51d1ee`) hide the zodiac
    sign control (`[esi + 0x1c4]`) for pets (`person.data[65] > 1`),
    suppressing `Kind::Sign` and `Help::Sign` balloons.
- **Dog & Cat Pet Skills subpanels (`0x51d598..0x51d624`,
  `0x51cbf8..0x51cc5a`, `0x531469..0x531bc3`, `0x531e43..0x5326d5`):**
  - When opening or refreshing panel 3 (`Panel::Job`), `0x51d5af..0x51d5bf`
    first routes child school age (`age != 0 && age < 18`) to the Report
    Card panel (`[esi + 0x124]`), then checks `person.data[65]`:
    - Bit 1 (`(kind >> 1) & 1 != 0`, Dog) shows `[esi + 0x114]` (`0x531469`):
      `Live.iff STR# 171[1..=3]` = `["House Breaking", "Tricks", "Obedience"]`.
    - Bit 2 (`(kind >> 2) & 1 != 0`, Cat) shows `[esi + 0x118]` (`0x531e43`):
      replaces row 0 with `STR# 171[4]` (`0x532262..0x532285`) =
      `["Hunting", "Tricks", "Obedience"]`.
  - Both panels sit at the standard subpanel origin (`0x6f3ad0`), draw
    title `"Pet Skills"` (`STR# 171[0]`) at `(13, 8)` in font 11
    (`0x531b7a..0x531bbf`, `0x5326a9..0x5326d4`), draw three right-aligned
    font-7 labels in `102 x 16` boxes at `(103, 25 + 15 * row)`
    (`0x6f4518`, `0x5317cd..0x531851`, `0x5321a7..0x53222b`), and draw
    three skill bars at `(207, 27 + 15 * row)` (`0x6f4488`) using
    `id::SKILL_EMPTY` (`0x1338` = 4920) and `id::SKILL_FULL` (`0x1325` =
    4901) from `person.data` words `[10, 12, 11]` (`0x66e360`,
    `points = (value / 100).clamp(0, 10)`).
- **Ultra-speed 500 ms blink (`state 3`, `0x46d7b0..0x46d812`,
  `0x4f6842..0x4f689d`, `0x53a5e3..0x53a6e7`, `0x53b4b1..0x53b7e7`):**
  - Entering idle speed (`0x46d790`) sets global 16 to `IDLE` (`1000`),
    saves the return speed in global 18 (`>= 0`), and sends message `0x109(1)`
    which sets `[control_panel + 0x1ec] = 1` and invokes `0x53b536(2)`.
    Leaving idle speed (`0x46d7db`) sends `0x109(0)` (`+0x1ec = 0`), and
    clicking Normal/Fast/Ultra in Live mode clears `+0x1ec = 0`
    (`0x53a5e3..0x53a6e1`) while updating global 16 away from `IDLE`.
    Thus `Snapshot::idle_speed` tracks
    `world.return_speed() >= 0 && world.speed() == sims_sim::speed::IDLE`.
  - When unpaused in Live mode with `idle_speed` active and saved return
    speed != `ULTRA` (`0x4b4513 != 0x46d26a`), the clock blink state
    machine enters `state 3` (`ClockBlinkMode::Ultra`), arming the shared
    500 ms (`0x1f4` ms) timer (`0x53b6e0..0x53b705`). Each callback
    (`0x53b4c9..0x53b4fe`) toggles `id::ULTRA`'s `selected` flag
    (`[ultra + 0xe0]` via `0x5e480a`) while the saved return speed button
    (Normal or Fast) remains highlighted (`0x53b0d8..0x53b16b`). Leaving
    state 3 cancels the timer and clears `ultra_lit` (`0x53b76d..0x53b7aa`).
- **Neighbor/GUID selection matching (`0x51c5ef -> 0x4e97dc`, `0x4e986d`,
  `0x4e9608`, `0x4cf9ea..0x4cfa0d`) & non-residential House suppression
  (`0x51d3f9..0x51d40f`):**
  - `Snapshot::selected_guid` records the selected person's type GUID so
    `Snapshot::selected_resident` matches the household roster by GUID
    (matching the executable's neighbor-index comparison) even when a
    second live instance of a household Sim is selected; `Game::select`
    likewise compares the selected Sim's GUID against the portrait's Sim
    so clicking the highlighted portrait centres on the first instance
    instead of switching selection (`0x4cf9ea..0x4cfa0d`).
  - On non-residential lots, `hud::live_strip` suppresses `Panel::House`
    selection and rendering (`0x51d3f9..0x51d40f`) in addition to
    `check_press` refusing clicks with `UI_error` (`0x51bdf2..0x51be36`).

## Ratchet impact

Zero pixel changes on all eight ratchet scenes (all eight scenes select
adult human Sims in the residential Goth house with normal speed).

## Not ported / what would reopen this

- Needs / Pet Needs / Pet Skills help balloons (`STR# 130`, `STR# 180`,
  `STR# 172`) and their object-thumbnail balloon layout (`0x5100db`).
- Child Report Card (`[esi + 0x124]`) and Superstar (`[esi + 0x11c]`)
  subpanels on the Job tab (`0x51d5af..0x51d616`).
- `sims-viewer` live 500 ms blink timer loop (viewer renders static
  world snapshots without a wall-clock tick loop).
