# The personality panel draws its bars as the original's bar maker lays them out

*Decided 2026-11-15.*

The live panel's Personality button now opens the selected Sim's
personality in the subpanel, where Needs opens the needs
(`sims_ui::hud`, `Kind::Personality`). Without a selected Sim the
subpanel is empty, as for the needs.

- **Where the numbers come from** (no screenshot of this panel was
  available, so it is read from the exe):
  - The panel builder (0x518665) makes five bars with 0x51df31, for
    person data words 7, 6, 3, 5 and 2 (the table at 0x66c600): Neat,
    Outgoing, Active, Playful and Nice.
  - Every bar's label is as wide as the widest label plus 3 (0x52144a),
    and its bar starts 3 past that (0x521491).
  - The bar's gadget is 3 wider than the bar, and the gadgets are
    stacked right-aligned at x 202 of the subpanel, from y 8, 18 apart
    (0x519cc8). The CJK languages use 15 and 25, which are not ported.
  - The shorter of the label's line and the bar is centred on the
    other.
- **A bar** (0x5211bd) draws the empty bar (4903, 40 by 14), then the
  full one (4900, 40 by 16) cut to 4 pixels a point and a pixel higher
  (0x5212b2). A trait is the data word over 100, clamped to 0 to 10.
- **Labels and fonts:** the labels are font 8 (0x6f2ae4) and the title
  is font 11 at the needs title's place. The labels are English, as the
  needs' are (Live.iff STR# 131 has them).
- **The subpanel** is the needs' rectangle. The layout table at
  0x6f3a28 puts it at x 300 of the strip, one left of the 301 the needs
  were measured at. That pixel is left as measured.
- **Unverified:**
  - the labels' color (the needs labels' is taken) and their alignment
    (right, against the bar);
  - which of the font's heights the centring compares (its line height
    is taken).
- **Not drawn yet:** the zodiac sign and its text under the bars (a
  text gadget in font 12, filled from UIText.iff STR# 165).
- **Tests:** a unit test checks the layout, presses passing through,
  each bar's fill and its rows with stand-in art, and that coordinates
  at the edge of `i32` do not panic. The Goths' panel is pinned by
  digest, gated on `SIMS_INSTALL`, with Mortimer's, Bella's and
  Cassandra's traits read from the house.

Reopen with a screenshot of the original's personality panel.
