# Save/Autosave Manager UI, Travel Shop/Wardrobe Picker & Cheat Console

- **Date:** 2026-11-21
- **Scope:** `crates/sims-ui/src/saves.rs`, `crates/sims-ui/src/shop.rs`, `crates/sims-ui/src/cheat_console.rs`
- **References:** The Sims Complete Collection 1.91 (`UIGraphics` `3008` / `17` / `5107`, `GameData/UIText.iff` `STR#` 142 / 220 / 221, `cTSWinTextEdit2` `rgb565(0x000a)` / 500 ms caret blink) and `docs/vision.md` (**Saves**, **UI and presentation**)

## 1. Save & Rotating Autosave Slot Picker Dialog (`crates/sims-ui/src/saves.rs`)

- **Native 1.91 Chrome & Zero Coupling:**
  - Uses `PopupInfoTiles.bmp` (`3008`), `WinBtn.bmp` (`17`, horizontally 3-sliced), and `HotspotPopupTiler.TGA` (`5107`).
  - Decoupled from REDB persistence via read-only `SaveManagerSnapshot` / `SaveSlotSummary` and typed `SaveManagerCommand` (`LoadSlot`, `DeleteSlot`, `FallbackToNewestIntact`, `Cancel`).
- **Deterministic Ordering & Non-Destructive Corrupt-Save Recovery:**
  - Displays manual save(s) first, followed by Track 3's rotating REDB autosave slots ordered newest-first by `(tick, saved_at_epoch_secs, rotation_index, slot_id)`.
  - Each row renders its slot label, optional household name, sim day/time/tick, and color-coded `SaveHealth` badge (`Intact`, `Corrupt`, `Unsupported`).
  - Per `docs/vision.md`, whenever any slot is corrupt/unsupported or `active_error` is set, the dialog displays a non-destructive warning banner clarifying that previous saves were preserved, disables `Load` on non-intact slots, and enables `FallbackToNewestIntact` targeting the highest-tick intact slot.

## 2. Travel Shop, Wardrobe, Pet Adoption & Magic Vendor Picker (`crates/sims-ui/src/shop.rs`)

- **Unified Destination Modal Picker:**
  - Covers Downtown boutiques (`WardrobeCategory::{Formal, Swimwear, Pajamas, Winterwear, Everyday}`), Vacation Island kiosks, Unleashed pet adoption displays, Studio Town wardrobe racks, and Magic Town spell/ingredient/MagiCoin vendors.
  - Scrollable item cards render aspect-preserving icon/portrait slots, item names with `[Preview]` / `[Owned]` tags, formatted prices in `§` Simoleons or `MC` MagiCoins, and wrapped descriptions.
- **Live Affordability & Localized `UIText.iff` Helpers:**
  - `Wallet::can_afford` enforces non-negative prices and per-currency balances (`Simoleons` vs `MagiCoins`); `ShopPickerDialog` disables `Buy` when unaffordable, already owned, unavailable, or negative-priced, and disables `Try On` for non-previewable items.
  - `ShopStrings::load` reads localized labels at runtime from `GameData/UIText.iff` (`STR# 142` default buttons, `STR# 220` clothing dialog text, `STR# 221` pet adoption strings) with generic technical fallbacks so zero verbatim game prose is embedded in source or tests.

## 3. Classic Sims 1 Cheat Console (`Ctrl+Shift+C`) Overlay (`crates/sims-ui/src/cheat_console.rs`)

- **Top-Left Overlay & Editing:**
  - Translucent/framed top-left bar (`5107`/`3008`) with a `cTSWinTextEdit2`-styled dark-navy input box (`rgb565(0x000a)`), 500 ms blinking caret, mid-string cursor movement, bounded input (`MAX_INPUT_LEN = 256`), up/down command history with draft preservation (`MAX_HISTORY = 64`), and a color-coded status/error feedback line.
- **Pure-Rust Command Parser:**
  - Parses classic Sims 1 cheats case-insensitively (`rosebud`, `klapaucius`, `;`-separated chaining with `!` repetition capped at `MAX_CHAIN_COMMANDS = 256`, `move_objects on|off`, `map_edit on|off`, `water_tool`, `autonomy <0..=100>`, `interests`, `prepare_lot`, `set_hour <0..=23>`, `sim_speed <-1000..=1000>`, `help`) and emits typed `CheatCommand` values for `sims-game` / `sims-vm`.
