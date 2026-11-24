# 2026-11-24: UI/Render Wave 4: the live subpanels to native pixel parity

## Context

Wave 4 audits the live-mode HUD against the 1.91 oracle tour
(`ui-oracle-tour-20261122`) and the 1.91 disassembly. It starts with the
four expanded live subpanels: Relationships, Job, Personality and House.
Their tour strips scored .9555, .9598, .9673 and .9495. The tour's diff
masks and a per-region shift fit traced every miss to one of six native
rules that we did not follow. Each rule below was read from the exe.
With all six in place the four strips score 1.0000 in the tour at
1024 by 768 (per-channel tolerance 40). Compared exactly, every pixel of
the four subpanels is within 7 of the original's in each channel. That
is under the step of the original's 16-bit 5-6-5 screen.

## Changes

1. **The subpanel rect.** Native puts the live subpanel at (0, 50, 280,
   150) in a holder at (300, 0) of the live panel (`0x517d9e`,
   `0x517e3d`). That is strip x + 300, 280 wide, with 224 more at
   1024 (`0x51f258..0x51f260`). Ours was at 521 and 279 wide, one
   pixel right. The measured offsets of the needs panel had absorbed
   that pixel, so they move +1 with it and the needs strips keep their
   scores. The rect is held to the gadget, 107 past its left edge, which
   is the strip's 300 at 800 and wider. The help hit test
   (`in_subpanel`) uses the same edge, so a window narrower than 800
   keeps the two together.
2. **The font's measure.** The font's width method (vtable `+0x68`,
   `0x60b7e2`) returns the sum of the advances plus `[font+0x34]`.
   `0x60bbc7..0x60bbda` sets that field to the glyphs' mean x bearing,
   in single precision, plus a half, truncated (`__ftol`). The result
   is 1 for the 7, 9, 11, 12, 14, 16, 18 and 20 point fonts and 0 for
   8 and 10. The draw-in-a-box (`0x60b4a1`) aligns by this width:
   - right alignment at right − width;
   - centring at left + (w >> 1) − width / 2, each half truncated
     (`0x60b574..0x60b58d`).

   New `Font::box_width` returns this width. `Font::width` stays the
   plain advance sum, because dialog layout, wrapping and carets were
   measured on native's screen with it. The line-fit method
   (`0x60b620`) does not add the extra. `box_width` is used for:
   - the skill labels (right aligned);
   - the relationship numbers (centred);
   - the house facts (right aligned);
   - the house rating labels, whose boxes are this width and end 3
     left of the bar (`0x52e0e2`, `0x52e10c`).

   The 8-point house font's extra is 0, so the Goths' panel shows no
   difference. A unit test with a font whose extra is 1 does.
3. **The text box's vertical centring.** The text box (`0x5e3f3c..0x5e3f78`)
   centres by the rows an `A` inks, not by the line:
   y = top + (H − (A bottom − A top)) / 2 − A top, truncated toward
   zero. New `Font::rows` returns those rows, or `None` for a character
   the font has no glyph for. `hud::text_box_y` applies the formula and
   centres the whole line when there is no `A`. It is used for:
   - the zodiac sign, a box of text width by line height at
     (5, H − line − 5) (`0x51cd11..0x51cdfe`);
   - the house facts;
   - the rating labels, line-high boxes moved only sideways
     (`0x52e0bb..0x52e115`).
4. **Relationships.**
   - The panel has a "Relationships" title (7-point font) that ours lacked.
   - The buttons start one pixel under a font-slot-6 line (`0x51dd1d`).
     The font table (`0x4ff546..0x4ff62b`) back-fills slots below the
     smallest font, so slot 6 is the 7-point font and its line is 13 high.
   - The page arrows centre below that line (`0x51ddec`).
5. **Personality bars.** Native compares the font's `A`-bottom (`+0x70`,
   `0x60b927`) with the bar image's height, not the line height
   (`0x5214e1`, `0x52152f`).
6. **The Layout rating.** `0x4166ee..0x41674f` sums the five saved days
   of usage samples and today's. With total samples T and routing
   samples R, Layout = clamp01((max(T, 1) − 2R) / max(T, 1)) × 100,
   truncated. The sums wrap at 32 bits. So does 2R, which is formed
   with `lea eax, [ecx + ecx]` (`0x416719`) before the `fild`. The samples are pairs at lot `+0x38..+0x64`, and the
   simulation already keeps them (`World::usage`). `house::layout`
   ports the formula in single precision. The Goths come out at 38,
   the three bars the original shows.

The measured offsets that remain are labelled as measured in the code:
- the needs, personality and relationships titles at (7, 2);
- the skill labels' box at y 5.

The table that sizes those boxes was not found.

## Evidence

- `ui_oracle_tour` floors for the four live strips rise from
  .94–.96 to .999.
- The install-gated HUD digests (`crates/sims-ui/tests/hud.rs`) are
  repinned. Each one that had no native comparison now cites the tour
  shot that matches at 1024, within 7 a channel.
- The sign balloon's rect is unchanged and `balloon.rs` is untouched. Its
  digest moves only because the personality panel under it does.
- Unit tests. Each fails with its fix reverted, which was checked by
  reverting each fix in turn:
  - `Font::box_width`, with a mean bearing of exactly a half;
  - `Font::rows`, including a font without an `A`;
  - `text_box_y`, including truncation toward zero and no `A`;
  - with a font whose extra is 1, the house facts and labels and the
    skill labels move a pixel left, and a centred "7" one pixel;
  - a personality label centred by the `A`-bottom;
  - the subpanel and its hit test agree at 640, 800 and 1024;
  - the relationships title and button column;
  - `house::layout`: the single-precision 90 at 18/20, a wrapped sum,
    and a wrapped 2R.

## Not in this change

- **The expanded puck at 1024 (.8655).** The puck art matches. The
  misses are the lot view behind it (camera and terrain), which belongs
  to the render track.
- **Portrait parity.** This covers the HUD roster, the dialog, the
  neighborhood family and the scrapbook. The tour holds no close-up of
  any of them that the audit could score, so it waits for captures.
- **Live polish:**
  - carried objects (after #413);
  - speech and thought bubbles;
  - headline icons;
  - Build/Buy placement feedback.
- **The build strips:**
  - the terrain sub-tool icons (+4, +6);
  - the separator;
  - the destination picker's title font and shadow.
- `sims_sim::World::usage` still documents that nothing ported reads
  it. `sims-sim` is outside this track's paths.
