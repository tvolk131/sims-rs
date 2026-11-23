# 2026-11-23: Travel destination top bar matches the 1.91 native bar

## Context

The user reported that the Vacation Island map's top bar "doesn't look like
the original". Our `sims-ui::travel` chrome drew:

- a destination switcher row (Downtown / Vacation / Studio / Magic);
- a second filter strip with four filter icons;
- invented "Hotel", "Rental" and "Camp" text tabs;
- a gold status badge ("Vacancy | $665", "MC n | $n", "Fame x.y");
- a "Fam" family-bin button docked by `sims-game`.

None of these has a source in `Sims.exe`. This audit checks every element of
the Downtown, Vacation Island, Studio Town and Magic Town top bars against
the 1.91 disassembly and the oracle tour (`cache/ui-oracle-tour-20261122`).

## Evidence (1.91 `Sims.exe`, image base `0x400000`)

**Construction.** Each destination class constructor takes one flag byte:

| Class | Constructor | Flag stored at |
|---|---|---|
| `cWinDowntown` | `0x549dae` | `[esi+0x128]` |
| `cWinVacation` | `0x5686e9` | `[esi+0x128]` |
| `cWinMagicland` | `0x551285` | `[esi+0x12a]` |
| `cWinStudiotown` | `0x5644b5` | `[esi+0x3b4]` |

- The neighborhood shell passes `1` (`0x4f5d7d`, `0x4f5f2c`, `0x4f60db`,
  `0x4f628a`).
- The lot-side callers pass `0` (`0x441503`, `0x441e28`, `0x44201b`,
  `0x4421ab`, `0x4443e8`, `0x44452f`, `0x4445a2`, `0x444615`).

**Flag 1 (the map screen).**

- The bar is `kDTBarBkg` 5422 (`NghUI/banner_downtown.bmp`, 800×52) on all
  four maps.
- It has no filter row and no title. The button loop is gated at `0x54a996`
  for Downtown and `0x569371` for Vacation.
- The loop builds one button table per class, in this index order:

| Class | Table | Buttons |
|---|---|---|
| Downtown | `0x6cd798` | 5209 Return, 5306 `kNghInet`, 5210 Sims logo, 5212 Import, 5211 Bulldoze |
| Vacation | `0x6ce368` | same, with Import 5214 |
| Studio | `0x6ce200` | same, with Import 5218. Return is the Downtown 5209, not 5217. |
| Magic | `0x6cdc9c` | same, with Import 5222, plus 5200 `kNghMoveIn` |

- The position tables (`0x6f53b0`, `0x6f62a0`, `0x6f5f58`, `0x6f5750`) are
  identical:

| Button | Position |
|---|---|
| Return | (323, 0) |
| Sims.com | (762, 0) |
| Logo | (5, 0) |
| Import | (422, 0) |
| Bulldoze | (373, 0) |
| Magic Town's Move In | (200, 0) |

- The logo is a 1-cell bitmap (`0x569445`); the other buttons have 4 cells.
- Tooltips follow table order: `STR# 169`, `170`, `173`, and `174` (which
  has 6 entries).

**Enable rules (`cWinVacation` mode switch, `0x56b19e`).**

- Browse mode enables Return, Sims.com and the logo.
- Import is enabled only when an import file exists (`0x56b870`).
- Bulldoze mode disables Return, Sims.com, the logo and Import, and holds
  Bulldoze down.

**Flag 0 (the lot-side picker).**

- `kFilterToolbar` 5045 (Studio Town uses 5046) plus `kDTDlgBkg2` 5432
  (`dlgframe2.bmp`).
- A random title from `STR# 212[0..3]`; Vacation uses `STR# 213`.
- A 7-button filter row at `x = 225 + 50·i, y = 20` (Vacation `0x56bf8c`,
  Magic `0x6f54d0`); Studio Town uses `175 + 50·i` (`0x6f5d40`).

**Oracle.**

- `flow-cab-downtown-picker.png` shows the flag-1 bar exactly: logo, Return,
  Bulldoze, greyed Import, Sims.com at the right edge.
- `flow-magic-hole-picker.png` shows the flag-0 dialog.

## Decision

1. **Every flag-1 element we draw is now native.** All four maps draw
   banner 5422 with their own button table at the native positions.
   - Sprites draw at their native origins. The window clips Sims.com's last
     column at x = 800.
   - Controls draw in table order, so Bulldoze covers Return's right edge
     and Import's left edge. Each hit rect is trimmed to its visible span,
     so a click lands on the button the player sees.
   - There is no text label on any button. Missing art falls back to a
     blank chip.
   - `Destination::bar_buttons()` cites the tables.
2. **Removed with no native source:**
   - the destination switcher (`TravelControl::SwitchDestination`);
   - the browse filter strip (`Destination::filters`, `filter_strip_rect`);
   - the Vacation lodging tabs (`VacationLodging`, `SetVacationLodging`);
   - the status badge (`format_status_badge`, `status_badge_rect`, and the
     `TravelState` fame, MagicCoin, Simoleon and vacancy fields);
   - the per-destination emblem and logo variants (`VACATION_LOGO` on the
     bar, `STUDIO_TOWN_RETURN`);
   - the `sims-game` "Fam" button.
   - Switching destinations stays on the neighborhood bar, which owns those
     buttons natively.
3. **Native controls that were missing are now wired:**
   - Sims.com shows the neighborhood bar's offline notice.
   - The Sims logo opens credits.
   - Magic Town's Move In opens the family bin. It replaces the invented
     "Fam" button.
   - Enable rules follow `0x56b19e`. Import stays greyed until lot import
     exists, which matches the oracle.
   - While a family is being placed, Move In is held down (`0x55478a`).
     Clicking it resets placement before reopening the bin (`0x552d96`).
4. **Lot pickers.**
   - **Magic Town** keeps its oracle-measured dlgframe2 dialog.
   - **Downtown, Vacation Island and Studio Town** keep the flag-1 bar.
     It holds no invented elements and matches the Downtown oracle (header
     score 0.9897). Only Return is live there, so a trip picker can neither
     edit lots nor open shell dialogs it cannot host.
   - The flag-0 dialog for these three is a follow-up. It needs a harness
     capture of a cab, shuttle or ferry picker, because the only Downtown
     shot shows the flag-1 bar.
5. **Ratchets.** `crates/sims-ui/tests/travel.rs` pins the result:
   - `destination_bars_hold_exactly_the_native_button_table`: the exact
     control set and positions for each destination.
   - `destination_bar_enable_rules_follow_the_native_mode_switch`.
   - `browse_bar_draws_only_the_banner_and_native_sprites`: with the real
     art, every bar pixel outside the button sprites must equal the banner,
     and nothing may be painted below the bar. A tab, badge or label that
     comes back fails this test.
   - The existing `ui_oracle_tour` Downtown header floor still applies.

## Follow-ups (not in this change)

- Flag-0 dialog pickers for Downtown, Vacation Island and Studio Town.
  Their filter orders are decoded above; Studio Town's IDs are partly
  decoded.
- Old Town's picker bar (`destination_picker::draw_old_town_bar`) still
  draws an invented title, `$funds` text and a tooltip.
- `TravelDialog`'s confirmation summary text ("Taxi -> Downtown",
  "Travelers", "Fare") needs its own native audit.
- Non-US languages swap the logo for `kNghSimsLogo*` (5421, 5427..5430;
  `0x54a561..`, `0x568f14..`). We always draw 5210.
- Lot import on destination maps is not ported. The button is drawn and
  stays disabled.
