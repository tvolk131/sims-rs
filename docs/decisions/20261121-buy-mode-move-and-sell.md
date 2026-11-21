# Buy Mode Hand Tool: Move, Rotate, Sell, Depreciation & Unified Undo/Redo

- **Date:** 2026-11-21
- **Scope:** `sims-vm` (`buy`, `boot`), `sims-render` (`canvas`, `scene`), `sims-ui` (`command`, `hud`, `buy`), `sims-game` (`game`, `hud`, `main`, `tests/goth.rs`)
- **Reference:** The Sims Complete Collection 1.91 (`Sims.exe`), Track 2 oracle recording `goth-buy-move-sell-20261121`

## Decisions

### 1. Hand Tool Pickup, Click-to-Pick-Up vs Drag-to-Drop & Grab Offset
- Picking up an existing lot object (`0x485f98` → `0x486960` → `0x4215e6`) resolves any multi-tile part to its group lead (`+0x190`), checks `CanPickUp` (`0x4352e8` / `0x422fdc`), captures its pre-pickup placement and all recursively contained surface children (`0x48ba19`), wraps it in a `"Tile Cursor"` (`0x00000437`), invokes `USER_PICKUP` (OBJf 12) + `take_off`, and immediately re-places both at the object's original tile/container so it starts valid on-lot (`before = cursor_id`).
- `CanPickUp` enforces `data[43] & 0x02 != 0`, `data[62] <= 0`, `data[8] & 0x20 == 0`, recursive `CanPickUp` on every non-empty surface child, and level-1 tile flag `0x20` clear. Persons are never pickable.
- Screen-pixel grab offset (`[tool + 0x50]`, `[tool + 0x54]`, `0x486960`) records `click_point - cursor_screen_point` on pickup and subtracts it before terrain unprojection in `0x485a2e` so objects picked by their upper sprite (e.g. a chair backrest) track the pointer without jumping a tile. New catalog purchases use `[0, 0]`.
- Native `[tool + 0x81]` (`buy_commit_on_release`): initial pickup sets it to `false`, so releasing within `< 2 px` (`dx^2 + dy^2 < 4`, `0x4864d0`) keeps the object attached to the cursor (click-to-pick-up); dragging `>= 2 px` drops it on release, and pressing with an already-active cursor sets it to `true` so release commits. Holding Shift while dropping a moved existing object never triggers repeat purchase (`+0x75 == 0`, `0x4864d0`).

### 2. Cancel Snapback, Sell, Depreciation & Unified Undo/Redo
- Cancelling a picked-up existing object (`Escape` / mode switch, `0x487191`) deletes only `"Tile Cursor"` and restores the held object and its surface children back to their pre-pickup placement via `0x48bfab` (`USER_PICKUP` → `take_off` → rotate → `UserTestPlace` → `put` → `USER_PLACEMENT`), with zero funds change and clearing the Redo stack (`0x4933a9`). Saving mid-pickup (`save_without_buy_cursor`) uses the same snapback on the cloned save world so an existing object is never orphaned or deleted on save.
- Selling (`Delete` / `Backspace` at `0x486b5b`, or clicking / dropping onto the Buy catalog strip at `0x50d04d` → `0x4cf97a` → `0x487e18`) checks `CanDelete` (`0x435440` / `0x423135`: `CanPickUp`, `data[43] & 0x08 != 0`, and all surface containment slots empty). If refused, `UI_error` plays and the cursor stays held.
- Resale / depreciation (`0x43219a`) reads the lead tile's `data[41]` (`+0x9c`) and halves it with signed integer division when any part in the group has `data[15] != 0` (`+0x68`, broken).
- Selling an existing object refunds `resale_value` via `spend(cat, -refund)`, deletes only `"Tile Cursor"`, leaves the sold object detached off-lot (`[-16, -16]`) in `world.objects` for Undo, and pushes a `BuyActionKind::Sell` record onto `buy_undo`. Selling an unpaid new catalog preview cancels and deletes both objects with `0` refund.
- Unified Buy Mode Undo/Redo (`0x48bd21` / `0x48bdfe`) handles all three action kinds:
  - **Buy (`kind 0`):** Undo detaches and refunds `price`; Redo restores and charges `price`.
  - **Move (`kind 1`):** Undo restores `before` (and recursive surface children); Redo restores `after` (and recursive surface children); zero funds change.
  - **Sell (`kind 3`):** Undo restores the detached object to `before` and re-deducts `refund`; Redo detaches and refunds `refund`.
- Leaving Buy Mode (`F1` / mode switch) or saving purges all detached off-lot instances referenced by Undo/Redo (`0x48c20e`).

### 3. Buy Mode Follow-ups Folded In
1. **Red-tinted blocked preview palette:** Blocked Buy previews drawn via `0x42242f` use `Canvas::draw_tinted` implementing `0x4b44eb(1)` → `0x4c76d1` → `0x4b9425(3, 0.9)` → `0x49bd78` (`gray = ((r+g+b)/3 * 0.9 + 0.5).trunc()`, `[min(255, gray + 0x30), gray, gray]`).
2. **Multipart `0x485f49` fit refusal:** `buy_move` keeps the saved `cursor.position` intact when fallback `fits(fine)` fails after unconditional `TakeOff(1)`, so manual rotation (`0x421be4`) rechecks at the saved tile.
3. **Live panel collapsed state:** `Game` preserves `live_panel_expanded` across Live → Buy/Build/Options → Live transitions (`0x4cea5d`) and re-syncs `view_centre` when `expanded` changes.
4. **Second Buy button click:** Clicking the Buy button while already in Buy Mode emits `Command::BuyToggleSort` (`0x53a2c3` → `0x4cf3af`), toggling `ByFunction` ↔ `ByRoom` instead of collapsing the strip.
5. **Buy Mode keyboard shortcuts:** Wired `F1` (Live), `F2` (Buy), `Delete`/`Backspace` (sell held), and `x`/`z` (next/prev catalog page, `0x4d0ef0`) in `sims-game`.
6. **Catalog popup card ratings:** `BuyItem` reads OBJD motive ratings `+0xa4..+0xb0` and skill flags `+0xb2` (`0x457a36`), formats them using `GameData/UIText.iff STR# 160` (`0x508168` → `0x50845d`), and renders them on the Buy popup card.
