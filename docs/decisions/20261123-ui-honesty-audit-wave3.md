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
   - The tests compare the port against three fixed permutations. They
     were produced offline by running the 1.91 routines (`0x501a69`, and
     `0x501b21` with an exhausted budget) under the unicorn x86 emulator
     over the shipped image, with the predicate call (`0x501c80`)
     answered from the same keys. No test runs the emulator, and none is
     gated on an install. The port leaves the same permutations, the heap
     fallback included; Sol re-ran the emulation independently and all
     three matched.
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
     - it shrinks by `min(37/w, 1)`, each side rounded to the nearest
       pixel.
   - The shrink is an exact area average: footprints come from the whole
     input and output sizes, with integer weights, and halves round up.
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
   - Each arrow steps the selection and tries the item on. The arrows
     wrap at both ends, as native's do (`0x40d229` past the last goes to
     the first, `0x40d360` before the first goes to the last).
   - The view opens on the person as they are (`0x56d3d9` copies them).
     OK and Cancel both send the index of the first item equal to the
     view's outfit (`0x5098b3`, `0x5098d1`; pets `0x522069`, `0x522087`):
     `0x56d852` dispatches on the view kind to `0x40d0a4` and its kin,
     which compare suit and texture, ignoring case (`0x472f92`). So the
     dialog opens with the first item equal to the person's current row
     selected and tried on. A repeated outfit answers its first index.
   - The row is the person's body string for the view kind, a 1-based
     slot through `0x472ec4`: everyday 1 (Wardrobe mode 0, Charm
     `0x5372d4` and the pet dialog `0x521fd8`, all kind 0), then formal
     30, swim 31, pajamas 32, winter 33 and high fashion 34.
   - When no item matches, the view shows the person as they are and no
     item is chosen. OK then answers `-1` (`ShopCommand::KeepCurrent`);
     the next arrow goes to the first item and the previous arrow to the
     last (`0x40d1bb` starts from `-1`). Cancel sends the shown index, or
     `-1`, as native does.
   - **Mode fix.** `0x5097c9` turns Wardrobe modes 0–5 into view kinds
     `0, 4, 2, 5, 0x11, 0x12`, whose lists filter by the letters at
     `0x6c0fac` (`B F S L W H`). Mode 1 lists formal and mode 2 swimwear;
     Wave 2 had them swapped. The `SpecialPanelRequest::WardrobeShop`
     doc in `sims-vm` still says 1 is swimwear; that crate is outside
     this track.
   - **Not native:** when the view cannot render (a pet without a
     skeleton, a missing mesh), the view names the item, so OK never
     accepts an unseen choice.
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
     crop in the household's album.
   - The photo is the house view alone (`Game::take_photo`), drawn
     without the panel, menus or dialogs, and framed at the click:
     - the frame sizes are the table at `0x6f0ef0` (`0x4cfb08`): Small
       200×150, Medium 400×300, Large 600×400; Custom is clamped between
       Small and Large (`0x4cfb5e`);
     - the click is recorded at `0x4ab80d`; `0x4b14a1` builds the frame
       `[x − w/2 − 1, y − h − 2, x + w/2 − 1, y]`, so it is centred on
       x and ends at the clicked row;
     - a frame past the viewport is shifted back in, left before right
       and top before bottom, then intersected with it (`0x4a76ed`);
     - the stored picture is the frame less its one-pixel outline
       (`0x4d1f6e..0x4d1f98`), so a Large photo is 598×400.
     The viewer's PNG copy (`LotHotkey::Photo`) has no click: it frames
     the last pointer position, else the viewport centre.
   - A save that fails shows a notice. Once a photo is stored, its ID is
     returned. An open album is then refreshed without moving its
     selection or dropping a caption being typed. A refresh that fails
     shows a notice, so a retry cannot store the photo twice.
   - A notice raised while the album is open is the shell's dialog,
     drawn above the album and answered before it (pointer, Enter,
     Escape and typed keys).
   - The album loads thumbnails for the strip the dialog draws. The
     shell's copy of the strip offset is now read from the dialog, so a
     refresh can no longer move the loaded window away from the shown
     strip.
   - The Camera strip's dead Take Snapshot path is removed:
     `CameraCommand::TakeSnapshot` and its button state.
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
   - Native has no presence test at all, so the filter is removed. A
     neighbor visiting the lot can be called, as in 1.91.
   - The native checks are mirrored:
     - `0x522ea1`: not the caller's family. Below family ID 2000, the
       family needs a nonzero ID and a house (`[family + 0x110] >= 1`).
       A neighbor is reached only as a member GUID of a family in the
       neighborhood's list.
     - `0x522f17`: kind ≤ 1, and the caller's first row for the neighbor
       (`0x464f74`) has at least one value.
     - `0x523011`: the neighbor's GUID resolves to a definition
       (`0x43ac10`). Ours uses the catalog through
       `Characters::definition`.
     - `0x52304c..0x523064`: the type's catalog name (`0x4573d9`) is
       not empty. Ours reads the Sim's display name (Make New Character
       names, then the catalog name the HUD loaded).
   - Each neighbor is listed once, judged by their first row.

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
- **Photo frame.** The frame's height is `h + 2` and its stored
  picture `w − 2` wide, as read from `0x4b14a1` and `0x4d1f6e`; no
  native photo was measured.
- **Choice dialogs' current row.** Ours reads the person's row from the
  strings Make New Character wrote, then the rows the clothing setup
  filled, then the type's `STR# 200`. Native reads its live string set
  (`0x4704fa`).

## Residuals

- **Door thumbnail colours.** The rendered door matches the oracle's shape
  (mean difference about 3/255), but native's colours are darker and
  greener, likely from lighting.
- **Roof pitch buttons.** They sit 3–5 px higher than the oracle's. The
  `STR# 147` y values (1, 30, 50, 71) do not reproduce it under the
  reading above, so we left them alone.
- **No oracle shot** shows the wardrobe, charm or pet dialog, or a second
  roof page.
- **Camera quality.** Native passes Low/Medium/High as 20/50/90
  (`0x4cfc53`) with the photo to the album writer (`0x4a9144` →
  `0x4ed79c`, entry `+8`), which hands it to its JPEG file writer
  (`0x4b7fb4`). The port stores album photos losslessly and has no JPEG
  encoder, so the setting changes no pixel. `SnapshotQuality::jpeg_quality`
  now gives native's numbers. Porting the encoder is left for later.
- **Camera frame outline.** Native draws the frame under the pointer
  while Camera Mode is open (`0x4b156a`). Ours does not draw it.
- **Phone book pages.** Native `cWinPhoneBook` shows one family per page:
  `0x5230cc` picks the page's family and `0x522f7a` lists its members.
  Ours lists every eligible neighbor in the caller's relationship order,
  as Wave 2 did. A follow-up should page by family.
