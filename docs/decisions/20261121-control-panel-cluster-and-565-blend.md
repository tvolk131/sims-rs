# Control-panel cluster layout, family-friends counter, and 16-bit 565 alpha blending

*Decided 2026-11-21.*

Inspecting the screenshot ratchet's panel diffs against the Complete
Collection 1.91 executable revealed six discrepancies in the live-mode
control-panel cluster (`crates/sims-ui`) plus the reason the ratchet's
`0816` scenes display `8:17` instead of `8:16`. Fixing the six UI
discrepancies brings every control-panel pixel of `far-r0-cut-0816`
outside the ratchet's blinking-pause-button mask
(`leave_out = 86 554 98 565`, 132 pixels hiding 72 differences) — 100%
of the clock prefix, family friends count, funds, speed button, help
button, and curved alpha border — to an exact 16-bit 565 match with the
original, leaving only the 49 pixels of the final digit `7` vs `6` (121
panel differences before applying the mask).

## Findings from the 1.91 binary

- **Clock format (`0x5fe51e`, `0x6d84a4`):** The English formatter
  starts at `0x5fe51e`, selects the executable literals `"AM"`
  (`0x6d84b4`) or `"PM"` (`0x6d84b0`), and formats the cluster clock
  with `"%s %2u:%.2u"` (`0x6d84a4`: meridiem, width-2 space-padded
  12-hour value, colon, zero-padded minute). Single-digit hours
  therefore carry two spaces between the meridiem and the hour digit
  (e.g. `"AM  6:30"`, `"AM  8:16"`), whereas double-digit hours carry
  one (`"PM 12:00"`). `Snapshot::clock` previously formatted
  `"{half} {twelve}:{:02}"` with one space.
- **Clock box and centering (`0x539ede..0x539f43`, `0x6f4578..0x6f4584`):**
  The clock text is drawn in font 8 (`[0x6f2ae4]`) in `GREEN` (`0x2f2b`)
  inside cluster-relative `(95, 118)..(151, 125)` (width 56, previously
  `at(75, 118, 100, 14)`), centered by signed truncation toward zero
  (`cdq; sub eax, edx; sar eax, 1`, i.e. `95 + (56 - w) / 2` via
  `Align::CenterTrunc`).
- **Why the ratchet's `0816` scenes show `AM  8:17`:** The live panel's
  clock reader (`0x4ce85f..0x4ce883`) directly formats `globals[0]`
  (hour, `[eax + 0x10]`) and `globals[5]` (minute, `[eax + 0x1a]`).
  `House05.iff` starts at `6:30:44` (22 ticks past `6:30:00`), so
  `ratchet.txt`'s `tick = 3189` (`3189 * 2 s = 1:46:18`) reaches
  `8:17:02`, two seconds past `8:17:00`. There is no UI clock bug; the
  49 differing pixels in `far-r0-cut-0816`'s panel (`x = 139..145`) are
  purely the final digit `7` vs `6`. `ratchet.txt`'s `tick = 3189` is
  kept unchanged so scene alignment with the baseline screenshots stays
  intact.
- **Family-friends counter (`0x539219..0x539346`, `0x4cfcdb`):** The
  `"0"` visible to the left of `"$2,542"` in the original screenshots is
  not part of the money string — it is a separate text control
  (`[esi + 0x1d0]`) displaying the household's family friends count
  (`"%d"` at `0x6c08e4`). It uses font 10 (`[0x6f2aec]`), `GREEN`
  (`0x2f2b`), right-aligned (`[eax + 0x164] = 2`), sized to
  `font10.width("999")` (`0x6cd398`) and placed immediately to the left
  of the money control (`[esi + 0x108]`), whose left edge is
  `178 - font10.width("$9,999,999")` (`0x6cd39c`, `0x6f4590 = 178`,
  `y = 0x6f459c = 158`). `hud::Kind::Friends(i32)` reproduces this
  placement.
- **Speed button selection (`0x53b0d8..0x53b16b`):** The control panel
  checks whether the idle loop's return speed is active (`0x46d7cc`,
  global 18 >= 0): if so it reads that saved return speed (`0x46d7d6`),
  otherwise it reads the active speed (`0x46d78b`, global 16), and
  matches against Normal (`0x2f8`), Fast (`0x39c`), or Ultra (`0x3e2`),
  clearing all three if none matches. It never inspects the pause flag
  (`0x46d866`, global 17), so a paused lot keeps its speed button lit.
  `hud::speed_for(world)` implements this check and both `sims-game` and
  `sims-viewer` now feed it into `Controls::speed` (`Option<Speed>`).
- **Help button (`kHelpBtn = 4993`, `0x1381`, `0x538f41..0x538fe6`, `0x6f4530`):**
  The cluster places `cpanel\Buttons\HelpButton.bmp` (40x15, four 10x15
  cells) at `(5, 164)` in its enabled/normal state (cell 0). Added to
  `hud::id::HELP`, `hud::RESOURCES`, and rendered via `Kind::Cell` with
  `command: None` (clicking to open the Help dialog at `0x50e8d3` is not
  yet ported).
- **Exact 16-bit 565 alpha blend table (`0x49a315`, `0x6e5eac`, `0x4986c6`, `0x4999ea`):**
  Both 16-bit 565 alpha sprites (`0x4986c6`) and antialiased FFN font
  glyphs (`0x60c446` palette `coverage * 17` -> `0x4999ea`) share the
  same 32x32x32 5-bit lookup table at `0x6e5eac` built once at
  `0x49a315` (averaging each 8x8x8 8-bit sub-bucket with
  `1.0f / 255.0f` at `0x6682d8` and `1.0f / 512.0f` at `0x6682d4`):
  `alpha <= 7` is skipped, `alpha >= 248` copies opaquely, and
  `8..=247` looks up `(a5 << 10) | (s5 << 5) | d5` per channel and packs
  `(r5 << 11) | (g5 << 6) | b5`. Replacing both the 8-bit linear blend
  in `Frame::blend` and the hand-fitted `ALPHA` table in `font::ink`
  with this exact table keeps the existing 20,000-pixel Magic Town
  dialog digest unchanged bit-for-bit while fixing every antialiased edge
  pixel on the control panel.
- **TGA un-premultiplication on load (`0x4de92c`, `0x4deb3d..0x4deb9d`):**
  When `[0x6c8c3c] == 1` (true for every UI TGA except the Magicland
  mist overlays `0x1655..0x1657`, which toggle `0x6c8c3c` to `0` at
  `0x54228f..0x5422d4`), the original's 32-bit TGA loader un-premultiplies
  partially transparent pixels (`0 < a < 255`) by `255.0f / a` rounded
  with `+ 0.5f` truncation in 24-bit single precision (`0x4fad5c`), then
  takes the low byte of each channel on store (`0x5dff82..0x5dff94`,
  wrapping rather than clamping — matching shipped pixels in
  `UniversalBack.TGA` at `(160, 75)` as well as `HotspotPopupTiler.TGA`
  and `NScreenDialogTiler.TGA`). `UniversalBack.TGA` has 393
  premultiplied edge pixels along the cluster's curved silhouette;
  un-premultiplying in `Art::load` and `Art::load_optional` eliminates
  the dark fringe around the cluster.

## Ratchet impact

Every scene in `crates/sims-game/tests/ratchet.txt` (relative to `origin/main` `a3e1c9bc`) improves by +2.22% to +2.54% on `panel` and +0.21% to +0.24% on `frame`, with zero regressions in any class:

| Scene | `frame` before | `frame` after | `panel` before | `panel` after |
| --- | ---: | ---: | ---: | ---: |
| `near-r2-0712-bench` | 96.51% | 96.73% | 97.33% | 99.76% |
| `near-r2-0805` | 95.39% | 95.63% | 96.79% | 99.33% |
| `far-r0-cut-0816` | 97.48% | 97.70% | 97.50% | 99.88% |
| `far-r1-cut-0816` | 97.49% | 97.70% | 96.40% | 98.75% |
| `medium-r1-cut-0816` | 97.87% | 98.08% | 97.50% | 99.87% |
| `near-r3-down-0816` | 96.60% | 96.82% | 97.50% | 99.88% |
| `near-r0-up-0816` | 97.67% | 97.89% | 95.80% | 98.15% |
| `night-far-r0-cut-2153` | 98.00% | 98.23% | 96.97% | 99.19% |

## Not ported / what would reopen this

- Clicking `kHelpBtn` (`0x53aaad -> 0x50e8d3`) to open the Help dialog,
  and hover state on `kHelpBtn` once it has a command.
- Hiding both the money and family-friends counters when `[ecx + 4] == 1`
  (`0x53b2a4`, `0x4cfe18`), and `0x4cfcdb`'s non-residential selected-Sim
  family lookup (`0x4e986d`, `0x4e9608`, `0x425d69`).
- Blinking of the pause button while paused.
- The 15-bit 555 display blend path (all comparisons use 16-bit 565).
- Skipping un-premultiplication for Magicland mist overlays
  (`0x1655..0x1657`) if those resources are ever loaded through `Art`.
