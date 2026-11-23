# 2026-11-23: UI Honesty Audit Wave 3: catalog order, rendered thumbnails, choice dialogs, album, roof arrows, phone book

## Context

Wave 2 (#401) closed with a list of known gaps. Wave 3 works through that
list. Each fix was checked against three sources: the 1.91 oracle tour,
the 1.91 disassembly, and the installed assets. The pet skeleton naming in
`sims_people` stays with the People track. `sims-game` keeps its
`load_pet_body` workaround.

## Changes

1. **Floor catalog order.**
   - `cWinArch` takes the price-and-name ordered list and re-sorts the
     Floor tool (type 7) by price alone (`0x5013ea`, key 0).
   - It uses MSVC's unstable `std::sort` (`0x501a69`). Floors with equal
     prices therefore land in the order that routine leaves them.
   - New `sims_ui::native_sort` ports the routine step by step:
     - introsort with a depth budget of `2·⌊log2 n⌋`;
     - median of three, then a Hoare partition;
     - a heap-sort fallback;
     - a guarded insertion pass over the first 16, then unguarded insertion.
   - `BuildCatalog::items_for(Floor)` runs it.
   - Wallpaper order is unchanged: once the swatches were fixed, its strip
     scored .98 against the oracle.
2. **Wallpaper swatches.**
   - Native swatches are the pattern's flat, camera-facing frame (frame 2,
     the diagonal wall), fitted into 37 px.
   - `hud.rs` now draws that frame with area sampling. It no longer shows
     a square crop.
3. **Door thumbnails.**
   - A definition with graphics but no kept `BMP_` gets a rendered
     picture, as native does (`0x4cc07e` → `0x4e57a6`). Doors slot 3 was
     blank because of this.
   - The object is drawn on a 200×200 surface filled with the
     ThumbTemplate's centre colours (`0x4cbfee`, one surface per cell).
   - Zoom 3, then 2, is used while a 3 px half-margin fits; otherwise
     zoom 1 (`0x438660`).
   - The direction is `(OBJD 0x7a + 4) % 8`.
   - `0x4e5aee` then reshapes the picture's rect:
     - it grows a side shorter than the 37 px cell to 37;
     - it squares the rect about its centre;
     - it clamps the rect to 199;
     - it shrinks by `min(37/w, 1)`.
   - The two cells sit side by side, so the result is 74×37 at full size.
   - `Thumbnails::with_template` carries the two background colours.
   - Three CC doors take this path: Outdoor Indoor Door, Gaily Gilded Gypsy
     Door and Threshold of Trepidation.
4. **Wardrobe, Charm and Pet choice dialogs.**
   - Native `cWinClothingChoiceDlg` (`0x508f22`, init `0x5090d0`) has no
     list, names, thumbnails, prices or Try On button.
   - It shows four things:
     - the `STR# 220[4]` title, "Please Select Outfit", for every mode;
     - a 200×300 `cWin3DView` (`0x509779`, param 3.25);
     - the 300/301 arrows (`DesignCharSkinsLeftBtn`/`RightBtn`, 18×34),
       9 px outside the view and vertically centred;
     - OK (`STR# 142[0]`) and Cancel.
   - The pet dialog (`0x521911`) has the same shape, with `STR# 221`
     ("Please Select Your Pet") and param 5.0.
   - `ShopSnapshot::choice` selects this layout.
   - Each arrow steps the selection and tries the item on. The dialog
     opens already trying on the first item.
   - Raw suit names and the " [Preview]"/" [Owned]" suffixes are gone,
     because no list exists.
5. **Pet preview scale.** Pets render at 200×300 and ×5.0/3.25 the Sim
   preview's scale.
6. **Photo Album.**
   - `STR# 141` names only six buttons: Done, Delete, First, Previous,
     Next and Last.
   - "Snap", "Add", `AlbumCommand::CaptureSnapshot`/`AddBlankPage` and
     `AlbumSnapshot::can_capture_snapshot` are removed.
   - Photos now come from where they come from natively: clicking the lot
     with the Camera strip open (`cWinCamera`).
   - `NeighborhoodShell::capture_lot_photo` stores the Camera Mode size
     crop in the household's album, and refreshes an open album.
7. **Roof page arrows.**
   - `cWinRoofPanel` (`0x52911a`) reads its six buttons from `UIText.iff`
     `STR# 147` `roofpanelstrs`. The art table at `0x66d73c` is
     `[3900, 3901, 3902, 100, 101, 3904]`.
   - Previous Page sits at `(62 − 4, 20)`.
   - Next Page sits at `(256 − 3, 20)` on an 800-wide screen, with 8
     patterns a page. Otherwise it sits at `(460 − 27, 20)`, with 16.
   - The panel is at `(GRID_LEFT − 20, strip_y + 5)`. Our pitch column
     (`10 − 4`) and pattern grid (`74 − 4, 3 − 2`) already sat there.
   - This puts Next Page at (971, 693) on 1024×768. The oracle shows it
     there, to the pixel.
   - Before this change we drew it at the strip's right edge
     (1010, 700), and fitted only 3 columns at 800.
8. **Phone book.**
   - Wave 2 left out any neighbor whose person object was registered on
     the lot. The aim was neighbors on this lot, but it also caught
     off-map ones.
   - Native has no presence test at all:
     - `0x522ea1` filters by family;
     - `0x522f17` checks kind ≤ 1 and a relationship row;
     - `0x523136`/`0x522f7a` require the neighbor's character to resolve
       by GUID. `0x43ac10` looks it up in the 5,454-entry object
       definitions table.
   - The filter is removed. A neighbor visiting the lot can be called, as
     in 1.91.

## Oracle tour

Changed rows; every other row is as in Wave 2.

| shot | region | Wave 2 | score | floor |
|---|---|---|---|---|
| buy seating | strip | .9918 | .9979 | .98 → .99 |
| wallpaper | strip | .8923 | .9842 | .88 → .97 |
| floor | strip | .9281 | .9635 | .92 → .95 |
| roof | strip | .9818 | .9866 | .97 → .98 |
| doors | strip | .9880 | .9925 | .98 |

`ui_wave2_snapshots` now draws the wardrobe, charm and pet panels as
choice dialogs. All three were inspected by eye.

## Unverified

- **Thumbnails.**
  - `0x4ba778`'s filter, taken here as an area average.
  - Its output rounding, taken as the nearest pixel.
  - The `[ebp-0x67]` flag `0x4e5886` passes for the second cell, taken
    as changing nothing but the background.
- **Choice dialogs.**
  - The size of the frame and its border.
  - The pet view's size; the dialog-area source is not traced.
  - Whether the arrows wrap at either end. Ours stop.
  - That Charm Transform (kind 14) uses `cWinClothingChoiceDlg`. This is
    inferred; the call path through `0x4417xx` would not disassemble
    cleanly.
  - The turntable. Native's `cWin3DView` spins; ours shows a still pose.
  - What the `cWin3DView` parameter (3.25 for Sims, 5.0 for pets) means.
    We treat it as a zoom and scale pets by 5.0/3.25. If it is a camera
    distance, pets should be smaller, not larger.
- **Roof pages** wider or narrower than 800/1024 use the 16-pattern layout
  when it fits, else the 8-pattern one, else our old fitted grid. Native
  only knows 800 and the rest.
- **Phone book.** The family check's `[family + 0x110] ≥ 1` test for IDs
  below 2000 is not mirrored. We test `family > 0`.

## Residuals

- **Door thumbnail colours.** The rendered door matches the oracle's shape
  (mean difference about 3/255), but native's colours are darker and
  greener, likely from lighting.
- **Roof pitch buttons.** They sit 3–5 px higher than the oracle's. The
  `STR# 147` y values (1, 30, 50, 71) do not reproduce it under the
  reading above, so we left them alone.
- **No oracle shot** shows the wardrobe, charm or pet dialog, or a second
  roof page.
