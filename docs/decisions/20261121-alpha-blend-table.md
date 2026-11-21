# Sprite alpha blending uses the original's 5-5-5 lookup table

*Decided 2026-11-21.*

The screenshot ratchet (`docs/decisions/20261120-screenshot-ratchet.md`)
showed that object match rates dropped sharply on anti-aliased sprites
(trees, bushes, hedges, windows, gravestones, cars and the bus — down to
74–84% at far and medium zoom), with nearly every differing pixel off by
one step in 5-6-5 and every differing tree pixel having an even 6-bit
green value in the original screenshot. The original's SPR2 object
blitters and blend table were read from `Sims.exe` rather than fitted.

## What the original does

- **Alpha cutoff (`0x4bbab8` at `0x4bbbe4`, `0x4bc080` at `0x4bc13f`,
  and the clipped/flipped variants `0x4bc606`, `0x4bca26`, `0x4bd590`):**
  SPR2 chunks are loaded raw into memory (`0x4c1757` via `0x5a93e0`),
  keeping `op == 2` pixel alpha bytes in `0..=31`. The blitters skip
  pixels with `alpha <= 7` (neither colour nor depth is written) and
  treat `alpha >= 0xf8` as opaque. That confirms the previously fitted
  `FAINT = 65` (`8 * 255 / 31`) threshold from code.
  Opaque `op == 1` / `op == 6` runs (`0x4bbce3`) write the lit 16-bit
  RGB565 palette colour directly, keeping all 6 bits of green.
- **Blend lookup table (`0x6e5eac`, filled once by `0x49a315..0x49a41c`):**
  A 32 × 32 × 32 byte table indexed by `(alpha5 << 10) | (src5 << 5) | dst5`.
  For each 5-bit triple `(alpha5, src5, dst5)`, it averages over all
  8 × 8 × 8 = 512 8-bit triples `(d, s, a)` in
  `[dst5<<3..+8) × [src5<<3..+8) × [alpha5<<3..+8)`:
  `blended = ((d + (s - d) * (a * (1.0f32 / 255.0f32)) + 0.5) as i32) as u8`,
  sums those 512 bytes, multiplies by `1/512` (`0x6682d4`), adds `0.5`
  (`0x666578`), truncates (`_ftol`), and shifts right by 3 (`0..=31`).
- **5-5-5 blending into 5-6-5 (`0x4bbc04..0x4bbc7d`, `0x4bc2eb`,
  `0x49a60b`):** Every RGB565 alpha blitter shifts both source and
  destination 6-bit green right by 1 (`g6 >> 1`), looks up
  `0x6e5eac[alpha5][src5][dst5]` for R, G and B, and packs
  `(r5 << 11) | (g5 << 6) | b5`. As a result, bit 0 of 6-bit green is
  always 0 on every alpha-blended pixel.

## How it is modeled

- `sims_render::canvas::blend565(src, dst, a)` lazily builds the exact
  32 KiB `0x6e5eac` table (`OnceLock`) using `0x49a315`'s formula,
  indexes with `a >> 3` and the top 5 bits of each channel, and widens
  `(r5, g5 << 1, b5)` back to 8-bit RGB.
- For every 5-bit SPR2 alpha `k` in `0..=31`, `((k * 255) / 31) >> 3 == k`
  exactly, so `a >> 3` recovers the original 5-bit alpha without
  changing `sims_formats::sprite::Frame`'s 8-bit RGBA representation.
- `Canvas::write` keeps `*pixel = rgb` for `a == 255` (opaque pixels,
  preserving all 6 bits of green) and uses `blend565(rgb, *pixel, a)`
  for `a < 255`.
- Scanning every `SPR2` chunk across `GameData/Objects/Objects.far`
  (3,836,354 `op == 2` pixels) finds 744,238 pixels with alpha `0..=7`,
  3,092,116 with alpha `8..=30`, and 0 with alpha `31`.

## Measured

Screenshot ratchet (`crates/sims-game/tests/ratchet.txt`, over
`origin/main` `9b006a84`):

| Scene | `frame` | `objects` | `panel` |
|---|---|---|---|
| `near-r2-0712-bench` | 96.51% → **98.06%** (+1.55) | 92.34% → **99.58%** (+7.24) | 97.33% |
| `near-r2-0805` | 95.39% → **97.22%** (+1.83) | 93.05% → **99.69%** (+6.64) | 96.79% → **97.28%** |
| `far-r0-cut-0816` | 97.48% → **99.30%** (+1.82) | 82.50% → **99.13%** (+16.63) | 97.50% |
| `far-r1-cut-0816` | 97.49% → **99.01%** (+1.52) | 81.79% → **99.17%** (+17.38) | 96.40% → **96.62%** |
| `medium-r1-cut-0816` | 97.87% → **99.27%** (+1.40) | 84.47% → **99.97%** (+15.50) | 97.50% |
| `near-r3-down-0816` | 96.60% → **97.43%** (+0.83) | 91.09% → **94.39%** (+3.30) | 97.50% |
| `near-r0-up-0816` | 97.67% → **99.29%** (+1.62) | 92.87% → **99.93%** (+7.06) | 95.80% → **97.50%** |
| `night-far-r0-cut-2153` | 98.00% → **99.30%** (+1.30) | 74.18% → **96.61%** (+22.43) | 96.97% |

`surface` and `people` are unchanged across all eight scenes. All 24
pinned Goth-house digests in `crates/sims-render/tests/goth_house.rs`
were updated to the new blended output.

## Not modeled

- The hypothetical SPR2 `op == 2` pixel with `alpha == 31`: in `Sims.exe`
  `31 < 0xf8` still routes through `0x6e5eac[31]` (dropping green's low
  bit), whereas `Frame::decode` maps `31` to `255` (opaque). Zero such
  pixels exist in `Objects.far`.
- The anti-aliasing-off blitter set (`0x4bf310` when `[0x6f0189] == 0`)
  and 15-bit 5-5-5 mode (`0x4bd9db` when `[[0x6e48bc]+0x34] == 1`).
