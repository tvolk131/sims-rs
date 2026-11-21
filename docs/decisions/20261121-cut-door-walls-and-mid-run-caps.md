# Cut doorways draw no wall stub and cap their flanking wall ends

*Decided 2026-11-21.*

The screenshot ratchet (`docs/decisions/20261120-screenshot-ratchet.md`)
showed grey wall stubs drawn across every doorway in the three near-zoom
walls-down scenes (`near-r2-0712-bench`, `near-r2-0805`,
`near-r3-down-0816`), where the original shows flat floor and neat
cut-height end caps on the wall ends flanking each doorway. The wall-run
drawing code was read from `Sims.exe` rather than fitted.

## What the original does

- **Cut door-like walls are skipped in live and buy modes
  (`0x53e968..0x53ea2b`, `0x53d5f9`, `0x416920`, `0x4cf0c9`):**
  When cutaway marks a wall (`0x53f0cb..0x53f0dd`), any cut wall with
  style `3`, `5`, `6`, `15`, `22` or `23` (`0x53d7d9`) gets flag bit 3
  (`0x8`), setting its pass count (`[ebp - 0x30]`) to `2` at `0x53e968`.
  At `0x53e9ce`, when `passes == 2`, the wall is cut, and the mode
  controller (`0x416920`, `[ecx + 8]`) is non-zero (live or buy mode; `0`
  is build mode, `0x4cf0f6`):
  - For a window (`style == 22 = 0x16`), if the wall is not capped at an
    end (`flags & 6 == 0`), `passes` is reduced to `1` (`0x53e9fa`),
    skipping build mode's blue outline pass and replacing style `0x16`
    with plain wall style `1` at `0x53ead5..0x53eadc` so a plain cut stub
    is drawn under the window.
  - For door-like styles (`3`, `5`, `6`, `15`, `23`), `passes` is zeroed
    at `0x53ea1f` and the loop jumps straight to `0x53ed7b` — no wall
    stub is drawn across the doorway.
- **Mid-run trim and end caps along straight X/Y runs
  (`0x53de05..0x53e77c`):**
  For each interior (non-record) corner between consecutive segments
  `prev` and `curr` on a straight X or Y run (`0x53df95..0x53e184`), the
  run computes the corner's trim bitmask (`[ebp - 0x14]`) and a mid-run
  cap flag (`[ebp - 0x16]`):
  - `is_cut` (`[ebp - 0xd]`, `0x53de05..0x53de4e`) takes `curr`'s cut
    bit if `curr` is solid (`0x53d7b3`), else `prev`'s if `prev` is
    solid, else `false`.
  - When a cut door-like segment meets a non-door-like segment, or when
    a fence (`0x53cd2a`: styles `2`, `12`, `13`, `14`) meets a non-fence
    segment, the door/fence bit is cleared from the trim mask and the
    mid-run cap flag is set (`0x53dfe2`, `0x53e04f`, `0x53e068`,
    `0x53e0de`, `0x53e14b`, `0x53e164`).
  - When the flag is set and a single straight bit (`1`, `2`, `4` or
    `8`) remains (`0x53e3c0..0x53e77c`), the run draws the same 3-pixel
    (at far zoom, `3 << zoom`) plain-style (`0x53e652`) end cap as
    `0x53cd4e` does for a corner record, using the surviving segment's
    pattern (`0x53e4c5`), the run's seen-side light (`0x53e701`, capped
    at night when no room is present), and standing height if `curr` is
    door-like next to a standing `prev` (`0x53e3ec..0x53e415`), else
    `is_cut`.

## How it is modeled

- `Painter::walls` in `crates/sims-render/src/scene.rs` skips drawing a
  wall segment when `cut && caps::door_like(style)` (`0x53e9ce..0x53ea2b`),
  and `wall_image` only maps `cut && style == 22` to `PLAIN_STYLE`
  (`0x53ead5..0x53eadc`).
- `Corners::new` in `crates/sims-render/src/caps.rs` populates `c.slots`
  for all corners, runs the exact `0x53de05..0x53e4ca` logic on each
  straight X/Y pair at non-record corners (independently of any crossing
  diagonal), sets `c.standing` / `c.cut` and `c.cap` via
  `caps::cap(mask, 0, 0)`, and records the run's lot-order final segment
  (`cap_light`, matching `[esi + side*4 + 0x28]` from `0x53f95b..0x53fac7`).
- `Painter::cap` in `crates/sims-render/src/scene.rs` uses plain style 1
  (`0x53e652`), reads the run's stored room (`cap_light`, `0x53dc6d`),
  and caps room-less light through the setter (`0x53e701`) when
  `!c.record`.

## Measured

Screenshot ratchet (`crates/sims-game/tests/ratchet.txt`, over
`origin/main` `bf3f5cab`):

| Scene | `frame` | `surface` | `objects` |
|---|---|---|---|
| `near-r2-0712-bench` | 98.06% → **98.78%** (+0.72) | 98.65% → **99.67%** (+1.02) | 99.58% → **99.65%** (+0.07) |
| `near-r2-0805` | 97.22% → **98.13%** (+0.91) | 98.13% → **99.55%** (+1.42) | 99.69% → **99.79%** (+0.10) |
| `near-r3-down-0816` | 97.43% → **98.10%** (+0.67) | 98.58% → **99.58%** (+1.00) | 94.39% → **94.46%** (+0.07) |

All other five scenes' class shares are unchanged (the `night-far-r0-cut-2153`
house region is left out of shares, so its `ours` hash updates while all
shares remain identical). All 24 pinned Goth-house digests in
`crates/sims-render/tests/goth_house.rs` remain unchanged.

## Not modeled

- Build mode's door/window cutaway outlines (`mode == 0`, pattern 254 at
  `0x53eb4e`, gated by `0x53d5f9`).
