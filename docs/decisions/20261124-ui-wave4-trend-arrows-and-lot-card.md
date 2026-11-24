# 2026-11-24: UI Wave 4 part 3: needs-bar trend arrows and the neighborhood lot card

## Context

Wave 4 part 2 (#431) documented the needs bars' trend arrows (`0x50c66f`,
`0x50c2fa`) but did not draw them: what decides when a need is sampled
(`flags & 9` at `0x51d05c`) was not traced. It also left the dialog,
neighborhood and scrapbook portraits unchecked against the 1.91 oracle.
This change traces the sampling gate and draws the arrows. It also fixes the
neighborhood lot card, which was the one portrait surface that was wrong.

## Trend arrows

All addresses are in `Sims.exe` 1.91. The full account is in
`sims_ui::needs_history`.

1. **The pacer.** The interface's update message (built by `0x4ce0e4`) keeps
   a pacer at `+0x200` (`0x4732c0`; accumulate at `0x4732d4`/`0x4732de`;
   take a period at `0x473311`). Each frame, `0x4ce390` adds the millisecond
   clock (`[0x6e54e0]`) and sets flag 1 for each whole period.
   - Flag 8 is set when someone is selected (`0x4cf9c4`, at `0x4cfa57`).
   - All flags are set when the update is forced, on going to live mode
     (`0x4ce432`).
2. **The gate.** The live panel's update `0x51ce79` takes `flags & 9` at
   `0x51d05c`. When it is not 0, it samples every need control once
   (`0x51d149..0x51d166`), then again for each further whole period
   (`0x51d173..0x51d183`). Only then does flag 8 run the selection
   (`0x51d23f` → `0x51c5a6`). The selection points the bars at the new Sim's
   histories (`0x51c9f4..0x51ca78` → `0x513aa8`) while the Needs panel is
   open. Opening the panel does the same (`0x51d694`).
3. **The period.** The period follows the game speed: message `0xc2`, flag
   `0x40`, then `0x4cee89`, with constants such as `0x3cc30c31`. That is
   1000 ms at normal speed.
4. **Sampling and history.** `0x513b41` samples, and `0x513aa8` points the
   bars. The history is `0x4cb4a5`: mean at `0x4cb58a`, push at `0x4cb5d6`,
   trend at `0x4cb635`, with capacity 10 and limit 5 (`0x513aeb`).
5. **The bar.**
   - `0x50c5da` resets it and `0x50c66f` reads the trend.
   - Each slot fades with `0x50c492`, ramped by `0x473390`/`0x4733b7`.
   - `0x50c56c` gives the colour.
   - `0x50c2fa` draws (vtable `0x66b220`), and `0x50c818` gives the shapes.
   - The anchors come from `0x5133ee` via `0x50c2cd`.

The HUD runs this on the presentation clock: `Hud::step_needs_trend`, plus
`needs_sample_due` so the window loop draws a frame when a sample is due.
Static frames (`clock_us == 0`) neither sample nor draw.

**Kept differences.**
- The native panel keeps a second bar set for pets (`+0x134`). Ours shows
  the same eight motives for both, so one set is kept.
- The native meter is set only after a sample (`0x513c39`). Ours still
  draws it from the motive on every frame.

## The neighborhood lot card

Against the 1.91 neighborhood oracle shots (`02-nb-hover-goth`,
`05-nb-hover-newbie`), our card had three faults:
- it was opaque;
- it sat about 38 px above the native card;
- it showed the 2002 face icons with an outline, where the native card
  shows the framed 25×25 thumbnails.

1. **Placement.** The card is a tooltip owned by the lot window (cWinLot,
   vtable `0x670204`). Its slot `0xbc` (`0x5ddbda`) takes the owner's rect:
   - x is the cursor's x (`0x5d9a2f`), clamped into `[L, R − 1]`;
   - y is `T − h − 1`, or `B + 2` if that is below 3.

   The tooltip manager (`0x5da037`, state 1→2 at `0x5da0ed`) then pulls the
   card onto the screen (`0x5da2b0..0x5da31b`):
   - left < 0 → 0;
   - right ≥ W → `W − w − 1`;
   - the same for y.

   The card class (constructed at `0x54f5c6`, vtable `0x6703c4`, built by
   `0x54fa2a` and sized by `0x4cb040` at `0x550876`) re-pins itself at slot
   `0xc0` (`0x54fa0c`). So cWinLot's 75 ms slide (`+0xa8`, `0x54dce6`) shows
   nothing. This is `sims_ui::neighborhood::tooltip_origin`. The rect is the
   lot's screen rect, and the cursor is where the card was first shown.
2. **Translucency.** The card's paint (`0x54f939`) works out a level:
   `ftol(fade × 255.0f (0x667668) + 0.5f (0x666578)) − 0x145`, clamped to
   −255..255. The fade is the lot window's 300 ms ramp.

   The blend (`0x4ba004` → `0x498b41` for 565, `0x4986c6` for 555) adds the
   level to the tiler's alpha (`+0xd8`'s `+0x60`):
   - it skips the pixel at ≤ 7;
   - it copies the pixel at ≥ 248;
   - otherwise it mixes through `0x6e5eac`, which is already ported as
     `Frame::blend`.

   Settled, the level is −70, so the opaque centre of `HotspotPopupTiler`
   blends at 185. This is `popup_alpha_shift`. The tiler's colour and its
   alpha are nine-sliced separately (`0x550a1a` draws into `+0xd8`, and
   `0x550a9b` copies to `+0xd4`). The text and portraits go onto the card
   first, so they blend with it.
3. **Portraits.** `0x550b02..0x550c53` draws each member as a 25×25 image:
   - x is `25 + 25i`;
   - the bottom row is at `h − 25`;
   - no outline is drawn.

   The source is `0x4e9556` → `0x4e9260(person, 0x7d4)`, which is BMP_ 2004.
   That is the character's 25×25 thumbnail, framed on black. `Faces` now
   loads it (`household::THUMBNAIL`). The face icon is the fallback.

**Kept differences.**
- The hover delay stays 275 ms. Native `+0xa4` is 0, but its trigger path
  differs.
- Native draws a second portrait row without a bound. Ours caps it at the
  row's width.
- Native still advances x for a member with no thumbnail. Ours closes the
  gap.

**Other portrait surfaces.**
- The cab and magic-hole dialogs show object icons, not portraits.
- The oracle has no scrapbook capture.
- The live roster portraits already score clean (#431).

## Verification

- `ui_oracle_tour::needs_trend_arrows_match_the_oracle`: the 800 Needs shot's
  three red arrows and the blue unlit slots, pixel for pixel.
- `needs_trend_arrows_show_as_the_lot_runs`: arrows light and settle to the
  darker red as the window loop runs.
- `ui_oracle_tour::neighborhood_lot_cards_match_the_oracle`, compared at
  ±1 per 5-6-5 channel:
  - lot 5: card .9845, portraits .9568;
  - lot 7: card .9848, portraits 1.0000.

  The remaining misses are the map's animated water under the card's edges.
  Each fault put back fails the test:
  - opaque card → .04;
  - BMP_ 2002 faces → .16;
  - anchor placement → .53.
- `sims-ui` `tests/neighborhood.rs`: `tooltip_origin` and
  `popup_alpha_shift`, on synthetic rects.
