# Pie menu entries are chosen on left release, not press

*Decided 2026-11-21.*

Closes the open item from `20261120-choosing-a-pie-entry-checks-it-again.md`
(#290): the window now chooses pie entries on left-button release rather than
press, so pressing an entry and dragging off the pie before releasing orders
nothing.

## What the original does

The people panel creates its two pie menus (`cTSPieMenu`, vtable `0x674b54`)
in `0x518665` and clears their button-down-select flag (`+0x170`) via
vtable `+0x24c` (`0x5ea7e2(0)`).

- **Opening (`0x48d56c`, `0x5eba03`, `0x5ebab2`):** `0x48d56c` reads
  `GetAsyncKeyState(VK_LBUTTON)`: when the button is still held (as when
  the menu opens on a left press), `+0x14e` (`sticky`) starts false; when
  the button is already up (as when reopening for a submenu or returning
  from one after a release), `+0x14e` starts true. If the menu had to be
  clamped to stay on screen, `+0x154` (`moved`) is set.
- **Left press (`0x5eb3d1`, vtable `+0xe4`):** with `+0x170` clear, it only
  calls mouse-move (`+0xfc`, `0x5eb4d7`), updating the highlighted entry
  without choosing, closing, or re-running the check on an already-hovered
  entry. While a mouse button is held, `0x5eb04c` draws the highlighted
  label in cyan (`PRESSED`).
- **Left release (`0x5eb433`, vtable `+0xf4`):**
  1. Calls mouse-move (`+0xfc`, `0x5eb4d7`) at the release point, updating
     the highlighted entry and distance (`+0x164`, 32-bit float from
     `0x5eb7c9`).
  2. If `+0x14e` (`sticky`) is false, checks `!moved && DEAD_ZONE < distance`
     (`35.0 < distance`, `0x5eb45d`–`0x5eb474`). When false, sets `+0x14e`
     to true so the menu stays open for the next click (settling the
     opening press). When true, proceeds directly to choose (single-gesture
     press-drag-release).
  3. When choosing, calls vtable `+0x1ac` (`0x5eb62a`) with `(hit, 1)`,
     which sends event `0xe` (`0x48df28`, checking the entry's action again
     when `hit` is a leaf choice) followed by event `0xf` (`0x48dd67`,
     queueing the action or opening a submenu) or event `0x10` (closing, or
     returning from a submenu when within `DEAD_ZONE`).
  4. Clears `+0x154` (`moved`) to false (`0x5eb4b3`).

## What changes here

- `menu::Menu` tracks `sticky` (`+0x14e`), `moved` (`+0x154`), and `held`
  (passed to `Pie::draw` for the cyan pressed highlight, `0x5eb04c`).
  `Menu::press` only marks `held`; `Menu::release` implements `0x5eb433`
  and returns `menu::Release::{Settled, Chose(usize), Stays, Closed}`;
  backing out of a submenu restores the parent's placement and warps the
  pointer back to the parent's middle when the parent clamps (`0x48db38` ->
  `0x5ebab2`); `Menu::release_outside` handles a release after the pointer
  leaves the window.
- `Game::press` on an open pie menu (with no dialog over it) updates hover
  and highlight as mouse-move does and marks the button held without
  choosing, re-checking an already-hovered entry, or clearing the pie's
  status/diagnostic note.
- `Game::release` runs mouse-move at the release point, settles an opening
  release inside the dead zone or on a moved menu (preserving any fallback
  status note), or else runs the second check (`Game::check_entry`) on the
  entry under the release point and carries out the choice, submenu
  transition, or close. `Game::release_outside` settles a moved opening
  menu or closes the pie without checking or choosing any entry. Pressing
  on an entry and dragging off the pie (or outside the window) before
  releasing now closes the menu without re-checking or ordering it.
- `sims-game`'s window shell (`main.rs`) checks physical client bounds on
  `CursorMoved` (clearing `cursor` and calling `Game::leave` when outside,
  just as `CursorLeft` does), forwards left-button releases to
  `Game::release` (or `Game::release_outside` when outside the client
  area), and moves the pointer when a submenu opens or backs out into a
  clamped parent. `Game::hover` and `Game::release` also route out-of-view
  points to `Game::leave` and `Game::release_outside`.
- The `goth-snack-bob-20261120` window replay in `tests/goth.rs` now sends
  the opening press at update 152, the opening release at update 154, the
  entry press at update 201 (pie stays open and held, nothing ordered),
  and the entry release at update 223 (ordering Have A Snack).

## Not verified / left out

- Right-button release (`0x5eb4c0`, which also forwards to `0x5eb433` in
  the original) and right-button cyan highlight remain routed to camera
  drag in `main.rs`.
- The pie window's own UI sound cues (`UI_piemenu_appear`,
  `UI_piemenu_highlight`, `UI_piemenu_select`), gated by `+0x17c`
  (initialized to `1` in `0x5ea303` at `0x5ea380`), remain unplayed here
  as before; only check-tree sounds are forwarded.
- Visual appearance of the cyan pressed highlight has not been compared
  against a screenshot of the original.
