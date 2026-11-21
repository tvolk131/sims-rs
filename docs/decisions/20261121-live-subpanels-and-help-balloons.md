# Live-mode Job subpanels (Report Card, Superstar / Fame) and help balloons

**Date:** 2026-11-21
**Status:** Accepted

## Context

Prior to this change, the live-mode control panel implemented Adult Job (`[esi + 0x110]`), Dog/Cat Pet Skills (`[esi + 0x114]` / `[esi + 0x118]`), and only the Zodiac Sign help balloon (`Help::Sign`, text-only). School-age children (such as Cassandra Goth) fell through to Adult Job on the Job tab, Superstar Sims with fame had no Fame subpanel, and the Needs / Pet Needs / Pet Skills / Report Card / Fame help balloons (including balloon picture slots) were unported.

## Complete Collection 1.91 verification

All behavior was disassembled from the Complete Collection 1.91 memory image (`capture-00000020-00400000.bin`) and verified against `GameData/Live.iff`, `GameData/fame.iff`, `ExpansionShared/ExpansionShared.far` (`work.iff`), and `UIGraphics/UIGraphics.far`:

1. **Job-tab subpanel priority (`0x51d598..0x51d624`, `0x51cbf8..0x51ccf4`):**
   - School-age child (`person.data[58] != 0 && < 18`) -> Child Report Card (`[esi + 0x124]`).
   - Dog (`(person.data[65] >> 1) & 1 != 0`) -> Dog Pet Skills (`[esi + 0x114]`).
   - Cat (`(person.data[65] >> 2) & 1 != 0`) -> Cat Pet Skills (`[esi + 0x118]`).
   - Superstar (`0x4cfe2c`: expansion mask `>= 0x40`, optional force-Fame flag `[+0x44c]`, `person.data[80] > 0`, and `0x4ea701 == 0` meaning no active regular career in `work.iff`) -> Superstar / Fame (`[esi + 0x11c]`).
   - Otherwise -> Adult Job (`[esi + 0x110]`).
   - *Note:* In the original, the live-notification refresh at `0x51d09a..0x51d143` checks `0x4cfe2c` directly rather than running the child/pet precedence first, and `[+0x44c]` is not modelled separately in the stateless snapshot resolver.

2. **Child Report Card (`[esi + 0x124]`, `0x532967..0x532f2f`):**
   - Title `Live.iff STR# 139[0]` (`"Report Card"`) drawn in font 10 (`0x6f2aec`) at `(panel.x, panel.y)` (western).
   - Grade button (`[esi + 0xf8]`, `0x532baf..0x532c4a`): font 18 (`0x6f2b0c`, `Fonts/variablesans_18.ffn`), full panel width, `font_18.line_height` tall, vertically centered in `panel`, horizontally centered text.
   - Grade lookup (`0x532dc0..0x532e0e`): checks `0x4ea722` (School career `-1`, level `0`), reads `person.data[57]` via `0x4ea88c`, maps via `0x4034ce` to 1-based index `grade + 1` of `work.iff STR# 4097` (`0x403f43 -> 0x6de7d8`, `"A+"` .. `"F (off to military school)"`, `""` out of range, or `"No cJob Object!"` at `0x6ccde8` if school career is missing).
   - Clicking `[esi + 0xf8]` (`0x532e66..0x532ef0`) toggles help balloon `[esi + 0x100]` (`Live.iff STR# 139[1..=2]`) with slot 0 set to row 11 (`0xb`, School) of `kJobPopupComposite` (4905, 22 rows of `84x63`, `0x530a7b(0xb)`).
   - *Not drawn:* the 333 ms blink for grades `>= 10` (`0x532de9 -> 0x5306df`, matching the Adult Job performance blink omission).

3. **Superstar / Fame subpanel (`[esi + 0x11c]`, `0x52b8cc..0x52d503`, `0x404d60..0x405415`):**
   - Loads `GameData/fame.iff` `STR# 1` (panel/rank/popup strings), `STR# 4` (friend star power needed), `STR# 5` (fame score needed), and `STR# 6` (6 comma-separated skill point targets per level).
   - Draws `"Fame"` in font 11 at `(5, 0)` (`0x6f3c38`), empty/full 5-star bar (`kFameSubBarsOff` 4823 / `kFameSubpanelBars` 4822, `80x16`) at `(3, 30)` (`0x6f3d58`) clipped to `fame_level * 8` px (full width at level 10, `0x52c8cc..0x52ca02`), top-aligned fame rank title in font 7 left-aligned in `(3, 45, 102, 16)` (`0x6f3eb0[6]`, `0x52c026`), 4-cell friends starpower button (`120x15` at `(3, 75)`, `0x52b87b`, `0x52c369`, icon `kFameFriendSmileyBMP` 4984 normal cell 0 / hover cell 2, `0x5e3be6`) + formatted text in font 7, and the 6 skills in `SKILLS` order with top-aligned right-aligned labels `(145, 6 + 15*n, 62, 16)`, bars `(210, 7 + 15*n)`, and underlined labels + L-shaped target marks where `person_skill < req * 100` (`0x405190`, `0x52cc11..0x52ccdf`).
   - Friend star power (`0x4ea6c4 -> 0x427237 -> 0x4271c5`): reads family (`word [neighbor + 0xee]`) and fame (`word [neighbor + 0x116]`) directly from each neighborhood record across mutual-friend neighborhood characters (`0x423f30`: disk characters and runtime-created `world.characters.made`) in a different non-zero family; formatted via `0x40529c` (`X.0`/`X.5`, without halving when `<= 0`) and `0x404dde` (`raw * 0.5 >= needed`, else `STR# 1[12]` `"%s (Need %d)"`).
   - Clickable help balloons: 6 skill labels (`Live.iff STR# 137[0..12]` with `0x6f3cb8[0..6]` object GUIDs), fame rank title (`fame.iff STR# 1[13..35]` with `0x6f3c60[0..11]` object GUIDs and bitmap sentinels `0x11 -> 278`, `0x12 -> 280`, `0x13 -> 279`), and friends total starpower (`fame.iff STR# 1[37..=38]`, no pictures). Out-of-range fame levels fall back to the constructor's initial level-1 text (`0x52c014`; in the stateful original, `0x52d402`/`0x52d462` skips updating the previous text).

4. **Needs / Pet Needs / Pet Skills help balloons, dismissal & picture-slot rendering (`0x50fc30..0x510697`, `0x4e5306 -> 0x4384c8`, `0x519603..0x5199c9`, `0x51f3c5..0x51f3e5`, `0x51d2f0..0x51d359`, `0x531850..0x532958`):**
   - Human Needs (`Live.iff STR# 130`, `0x6f39d0`/`0x6f39d4`, view overrides `0x6f3990`/`0x6f3994` forcing zoom 2 on Hunger right slot `0xff9f18e0`, Social uses `kSocialPopupIcon` 3750) and Pet Needs (`Live.iff STR# 180`, `0x6f3950`/`0x6f3954`, view overrides `0x6f3910`/`0x6f3914` forcing zoom 2 on Pet Hunger right slot `0xffc0ea58`, Social uses 3750) wire clickable `100x20` meter hitboxes at 19px vertical stride from `(0, 21)` (`0x513330`, `0x519a34..0x519a9e`).
   - Dog Pet Skills (`Live.iff STR# 172` pairs 0, 1, 2) and Cat Pet Skills (`STR# 172` pair 3 Hunting, then pairs 1, 2) wire clickable `102x16` label buttons at `(103, 25 + 15*n)`; `0x6f4458..0x6f446c` is all zero so neither uses pictures.
   - Balloon layout narrows text to `text_left = 168` when slot 0 is present (else `5`) and `text_right = 372` when slot 1 is present (else `535`). Slot vertical positioning follows `0x50fef5..0x50ff91` (including subtracting 50 from slot 0's effective height at screen width `1024`), and UIGraphics bitmap/row slots center via truncation toward zero (`0x5105cb..0x51065d`).
   - Object slots (`0x510547` type 1 -> `0x4e5306 -> 0x4384c8`) render the object's native isometric `DGRP`/`SPR2` (with `SPR#` fallback) sprites, including multi-tile assembly (`0x438bbc`, `0x43183c`, sorted by `(tile_y, tile_x)` at `0x438cc3`), default direction `(word(0x7a) + 4) mod 8` (`0x4385b7`), auto-fit zoom selection `3 -> 2 -> 1` requiring `>= 3` px margins (`0x438660..0x438702`) or forced zoom override (`[edi + 0x4c]`), centered and clipped to the slot (`0x4e5544`).
   - Moving the pointer outside the active subpanel (including onto the open balloon itself, which attaches outside `LivePanel` at `0x51a0a2..0x51a0b5`, onto the lot, cluster, portraits, gadget left of the subpanel, or leaving the window) or pressing outside both the subpanel and balloon dismisses an open help balloon via `0x50fb7f` and plays `UI_whoosh` (`0x51f1f2..0x51f3e5`, `0x51d2ee..0x51d359`). A direct left press on the balloon itself absorbs the press without dismissing (`0x66bd68 -> 0x5dd85f`). At width `1024`, the western subpanel stays anchored at `x = 521` and extends its right edge by 224 px (`0x518f40..0x518f48`, `0x51f258..0x51f260`, `0x532c08..0x532c20`). Balloons open and close immediately without the 250 ms slide animation.
   - All expansion/balloon bitmaps are loaded via `hud::OPTIONAL_RESOURCES` (`Art::load_optional`) so pre-Superstar installs still open cleanly.
