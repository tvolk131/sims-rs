# 2026-11-23: UI Honesty Audit Wave 2 — special panels, shell dialogs, and a UI oracle ratchet

## Context

Wave 1 (#397) fixed the core Live/Buy/Options gaps. Wave 2 covers the
special panels the VM opens through `SpecialPanelRequest`, plus the shell
dialogs (Photo Album, Travel, Import Family). These still drew generic
9-slice placeholders or leaked raw data. Wave 2 also adds a test that scores
our UI against 1.91 oracle screenshots and fails if any region drops below
its floor. Every fix after that test landed came from a discrepancy it
found.

## Changes

1. **Phone book (`Dialog` kind 10, `cWinPhoneBook`, `0x5221d0`).**
   - New `crates/sims-ui/src/phonebook.rs` draws the native
     `phonebookbkg.bmp` (80, 620×356) and `phoneicon.bmp` (81).
   - Each entry has a 45×45 portrait and Daily/Lifetime relationship bars,
     with paging.
   - Viewports smaller than 620×356 fall back to `PopupInfoTiles.bmp` (3008).
   - `CallNeighbor` wires the caller's neighbor record. The list holds the
     non-family neighbors the caller has a relationship with, so a fresh
     Mortimer Goth sees an empty book.
2. **Wardrobe (6), Charm Transform (14) and Pet picker (9).**
   - These pickers are now filtered by mode/category instead of listing
     everything, and they show no prices.
   - The pet picker previews the breed on the 3D turntable. Pet skeletons
     are `dog_skeleton.cmx.bcf` and `kat_skeleton.cmx.bcf`, with an
     underscore. `sims_people::Body::load` only looks for
     `<name>-skeleton.cmx.bcf`, so the new `load_pet_body` in
     `crates/sims-game/src/game.rs` falls back to the underscore name and
     binds the outfit the same way. The People track could fix this
     upstream.
3. **Import Family:** the modal no longer shows a stray `$0`. It shows the
   family funds and the search paths.
4. **Spellbook / reference book:**
   - Chrome is `magicbookbkg.bmp` (82), with illustrations 90..=94.
   - The title takes the caller's surname: "Goth Family Spellbook".
   - 82 is a plain blue 620×400 frame in the game data, so drawing it bare
     is correct.
5. **Photo Album:** draws the native Scrapbook art 1200..=1206 on viewports
   of at least 700×560. The six native buttons sit at their `STR# 141`
   coordinates.
6. **Travel dialog:** `PopupInfoTiles.bmp` (3008) frame, `WinBtn` 17 buttons
   and `STR# 142` labels. The Magic Town and Downtown pickers get the
   `dlgframe2.bmp` header and the filter row, measured against the oracle.
7. **P1-12:** removed the synthetic Take Snapshot button from the Camera
   strip. Native `cWinCamera` has none.
8. **Build Mode, oracle-driven:**
   - Catalog slots no longer mark unaffordable items. The native
     `build-stairs` shot shows plain cells.
   - Terrain subtools are in the native order Grass, Level, Lower, Raise.
     Grass is the default, and the icons are bare with no template cell.
   - Roof pitch column sits at `grid_left − 14` in the order Steep, Medium,
     Shallow, Flat. The pattern grid is at `+50`/`+1`.
   - Roof patterns are sorted case-insensitively by stem. The game matches
     patterns by stem, so re-indexing is safe.
9. **Pause:** the oracle's 1024×768 Live shot is paused. The tour pauses and
   advances one blink before it captures.
10. **Test hook:** `Game::preview_special_panel` (`#[doc(hidden)]`) opens a
    special panel over a given world without a VM request. Only the new
    snapshot test uses it.

## Oracle ratchet

`crates/sims-game/tests/ui_oracle_tour.rs` runs when both `SIMS_UI_ORACLE`
(the folder of 1.91 screenshots) and `SIMS_INSTALL` are set; otherwise it
skips.

- It drives the Goth house through Live tabs, view radios, the Buy and
  Build tools, Camera, Options, and the Magic Town and Downtown pickers.
- For each named region it scores the share of pixels within tolerance of
  the oracle.
- It fails if any score falls below its floor. Floors are
  `floor((score − 0.005) · 100) / 100`, so they only move up.
- `SIMS_UI_ORACLE_OUT` writes side-by-side PNGs.

| shot | region | score | floor |
|---|---|---|---|
| live 800 | puck | .9939 | .98 |
| live 800 | strip | .9979 | .99 |
| live 1024 | puck | .8655 | .86 |
| live 1024 | strip | .9997 | .99 |
| relationships | strip | .9555 | .95 |
| job | strip | .9598 | .95 |
| personality | strip | .9673 | .96 |
| house | strip | .9495 | .94 |
| view radios | radio | 1.0000 | .99 |
| buy top | strip | 1.0000 | .99 |
| buy seating | strip | .9918 | .98 |
| build top | strip | .9836 | .97 |
| terrain | strip | .9609 | .95 |
| water | strip | .9943 | .98 |
| wall | strip | .9678 | .96 |
| wallpaper | strip | .8923 | .88 |
| plants | strip | .9930 | .98 |
| floor | strip | .9281 | .92 |
| roof | strip | .9818 | .97 |
| doors | strip | .9880 | .98 |
| windows | strip | .9930 | .98 |
| stairs | strip | .9919 | .98 |
| fireplace | strip | .9943 | .98 |
| camera | strip | .9779 | .97 |
| options | strip | .9921 | .98 |
| magic picker | frame | 1.0000 | .99 |
| magic picker | header | .9118 | .90 |
| downtown picker | frame | 1.0000 | .99 |
| downtown picker | header | .9730 | .96 |

`crates/sims-game/tests/ui_wave2_snapshots.rs` opens every Wave 2 panel on
the Goth house with real art and asserts that it draws. With
`SIMS_UI_SNAPSHOT_OUT` set, it writes PNGs. All of them were inspected by
eye:

- phone book, empty and with 10 townies;
- wardrobe and charm;
- pet picker, with and without the 3D preview;
- Spellbook;
- Photo Album;
- Travel;
- Import Family.

## Known gaps (follow-ups)

- **Wallpaper (.89) and floor (.93):** our catalog order differs from
  native (we sort by price, then name, then id). Native wallpaper
  thumbnails are slanted wall pieces.
- **Doors:** slot 3 is blank. Native renders object thumbnails at
  `0x4e57a6`, which is not ported.
- **Live 1024 puck (.87):** the lot terrain and camera framing behind the
  puck differ. The puck itself matches.
- **Live tab values (relationships, job, personality, house):** some values
  differ from the oracle's moment in time.
- **Wardrobe and charm:**
  - The lists show raw suit names ("Dave", "Ct", "Flametroop", "Jrsk2")
    and no thumbnails.
  - Charm reuses the everyday category. This is unverified.
  - The selected try-on row gets a synthetic " [Preview]" or " [Owned]"
    suffix.
- **Pet preview:** the dog is drawn small at the bottom of the pane; the
  framing needs work.
- **Photo Album:** "Snap" and "Add" are not among the six native `STR# 141`
  buttons. They are our own extras with English defaults.
- **Roof page arrows:** the previous-page arrow sits at
  `pattern_left − arrow − 2`. No oracle shot shows a second page, so this
  is unverified.

- **Phone book:** the "is on the lot" filter treats every registered
  person object as present. A neighbor whose person object stays
  registered off the map would be left out (Sol review P3).

## Sol review

Round 1 (`gpt-6.2-sol`, high, at `4a49b4f4`) found `P0=0, P1=0, P2=3,
P3=5`, so there was no round 2. Fixed:

- **P2:** the native Visit picker layout dropped Vacation Island and Studio
  Town filters at 800 px and wider. It now applies only to the
  oracle-measured Downtown and Magic Town pickers
  (`Destination::picker_is_native`).
- **P2:** Magic Town's dialog trim let hover and clicks through to lots
  under it. Hover, press and release now use `TravelLayout::blocks_map`.
- **P2:** after a resize, the phone book could call a contact that was no
  longer on screen. The page is now derived from the selection, with a
  regression test.
- **P3:** a preview panel never answers a waiting VM request.
- **P3:** Magic Town's Family Bin button now includes the bar origin in
  its Y.
- **P3:** the snapshot test asserts that each panel paints and that the
  wardrobe and charm lists are not empty.
- **P3:** both new tests fail, instead of skipping, when an environment
  path is set but missing.

The off-map phone book P3 is left as a follow-up above.

The coordinator's Sol re-check of the fix commit `73481873` found `P0=0,
P1=0, P2=2, P3=2`. All four are fixed:

- **P2:** hovering the lot behind a modal ran pie-menu check trees, which
  changed the world. `Game::follow` now lists nothing while any modal is
  up. A test checks that the world hash is unchanged across a preview's
  open, hover, render and dismiss, and that the same hovering does list a
  menu once the modal is gone.
- **P2:** Enter or Escape on a picker preview could reach a VM prompt
  waiting behind it. `dialog_submit` and `dialog_confirm_key` now send
  keys to the foreground modal first (phone book, book, shop picker), and
  swallow them for other special panels and the destination picker. A
  test checks that the Goths' opening VM dialog stays pending behind a
  charm preview (Enter) and a phone book preview (Escape), and that Enter
  answers it once no preview is up.
- **P3:** whether a panel is a preview now lives on `ActiveSpecialPanel`
  and is set on every open or replacement. A real VM request that replaces
  a preview is no longer treated as one.
- **P3:** the snapshot test checks that the Photo Album's left bevel
  matches `scrapback.bmp` pixel for pixel (95% within 8 levels), so the
  9-slice fallback fails it.

A second re-check of `a7af8d6d` found `P0=0, P1=0, P2=2, P3=2`. It also
confirmed the album check: the fallback matches at most 926 of 4,200
pixels. All four are fixed:

- **P2: dismissing a preview could still change the world.** The pointer
  left over the preview made the next frame list the object under it.
  Closing a preview now clears the lot pointer, so listing resumes only on
  fresh pointer input. The test finds a point whose listing changes the
  world at tick 93, using a throwaway copy of the lot. It leaves the
  pointer there behind the preview, dismisses it, and checks that the
  hash is unchanged. It then checks that fresh pointer input on that same
  point does change the world.
- **P2: the community shop allowed lot checks behind it.** `follow` now
  also blocks while the shop is open. A separate test covers this with the
  same target point.
- **P3: a tick-zero request could be mistaken for the preview.**
  `ActiveSpecialPanel::shows` never matches a preview, so a real request
  replaces it. A unit test covers this.
  - The full VM round trip is not tested. No Goth-lot interaction raises a
    real special panel within a few thousand ticks, so there was no cheap
    fixture.
- **P3: typing reached the field behind the phone book.** The desktop
  handler gives text-entry keys to `has_dialog_entry` and the
  `dialog_*`/`type_dialog_char` methods. Those now all defer to whichever
  Game-drawn modal is in front.
  - The test puts a phone book preview over the edit-mode Save Card's
    text field. It checks that Enter, Escape, typing, Backspace, Delete,
    caret keys and Tab all stay with the phone book. Once the book
    closes, the field edits and Enter submits the typed name.
  - A scan of every interaction Mortimer can pick on the Goth lot found
    no kind 3 prompt. So the test uses the Save Card's
    `cTSWinTextEdit2`, which takes the same desktop path.

A third re-check of `0cc14d3e` found one P2 and one P3, both fixed:

- **P2: closing the F8 shop could still run a lot check.** `close_shop`
  now clears the lot pointer too. It does this every time, because Escape
  and the Cancel button close the dialog before the Cancel command arrives.
  The shop test now goes on through Escape and two redraws, checking that
  nothing is listed and the hash is unchanged. It then checks that a fresh
  hover lists again. Without the fix, the test fails at the redraw.
- **P3: the keyboard test's empty phone book is now deterministic.** The
  caller is a neighbor id that isn't in the world, so the book is empty
  whatever the saved neighborhood holds.

## Unverified assumptions

- Wardrobe modes 1–5 categories, and the item order for kind 14.
- NPCs are left out of the phone book by `family > 0`.
- The breed-name map.
- Native pickers show no prices.
- Magic Town's default filter is Magic.
- Downtown's omitted right button.
- Bulldoze is disabled while visiting.
- The interpolation of the picker header gradient.
- The first picker item is auto-selected.
- The OBJD Disabled flag (`+0x20`) applies only in Build Mode.
- Grass is the terrain default; the oracle shows no selection.
