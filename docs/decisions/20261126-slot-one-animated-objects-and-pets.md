# 2026-11-26: Slot-1 animated objects and what pets hold

*Decided 2026-11-26.* This extends
`20261123-person-held-objects-at-attach-points.md` and
`20261126-render-censor-held-order-walls-glide.md` (#486). It settles two
points that the first left under "Unverified and not ported": slot 1, and
pets.

## Slot 1: objects with a 3D graphic of their own

### What the original does

- **Which objects have a graphic.** When an object is made (`0x4376f6`),
  it gets a 3D graphic of its own if bit 0 of its definition's
  VitaboyFlags word is set. That word is OBJD word 102, byte `+0xcc` of
  the raw words at `[object+0x118]`.
  - The graphic is class `0x671d6c`, size 0x138, made by `0x58464f`. It
    is kept at `[object+0x120]` and initialised by vtbl[0] `0x584cb5`,
    and `[object+0x180]` is set to 1.
  - It reads its skeleton and suit from the type's body strings: entries
    1 and 2, counting from 1. The suit is the part before the comma.
  - In the install, only the three plumb-bob types have the bit. All three
    use the one-bone skeleton the bob already uses.
- **Where it is.** The tail of the person animator's tick (`0x58dc72`,
  tail `0x58f8ff`..`0x58fa91`) reads the person's slot vector entry 2
  (`[person+0x130]+0x48`, scripts' slot 1). It takes that entry's object
  (`+0x10`) and the bone name at `+0x20`. It stops if the object's
  graphic (object vtbl `+0x2c`) is null or there is no bone name.
  Otherwise:
  - It builds the graphic's matrix: identity (`0x5c3700`), translated to
    the named bone's position (bone `+0x4c..+0x54`, found by `0x5990e0`),
    times the person's matrix (animator vtbl `+0xb0`, multiplied by
    `0x462aab`). The bone's own rotation is not used.
  - It calls the graphic's vtbl `+0xac`, then `+0x64(1)`, then `+0xb8`
    with the matrix.
  - It stores the person's mood (motive 3, `0x45d117`, cut by `0x636128`)
    in the graphic's `+0x100`.
  - It ticks the graphic (vtbl `+0x50`).
- **Not drawn by the person draw.** The person draw (`0x459dc0`..
  `0x45a131`) draws only vector entries 1, 3, 4 and 5 (scripts' slots 0,
  2, 3 and 4). The generic slot walk (`0x437c56`) skips people. So nothing
  else in slot 1, or in a slot past 4, is drawn.
- **Mode.** The graphic compares its suit name byte for byte (`0x46f83e`)
  at `0x5856d4`..`0x5857a9`:
  - the standard suit gives mode 1;
  - the string at `0x6d05e4` gives mode 2 (the date bob);
  - the string at `0x6d05d8` gives mode 3 (the star bob).

  Each match sets the bob flag `+0xf8` and the mode `+0xfc`.
- **Colour.** The draw (`0x585dc5`, colour at `0x58603b`..`0x586165`)
  starts with r, g and b at 1:
  - Mode 3 is yellow (1, 1, 0) and does not read the mood.
  - Otherwise m is the mood × 0.012 (`0x671eb4`), held to −1..1.
  - Mode 1 is the existing rule: red, then pale yellow, then green.
  - Mode 2 is mode 1 with green and blue swapped: red, then pale pink,
    then blue.
  - The light intensity, the ambient light (a quarter of the colour plus
    0.05), the raise of 3, the spin and the Euler routine are the same in
    every mode.

### What we do

- `Cast::vitaboy_flags` reads OBJD word 102. `Cast::animated_suit` gives
  the suit of a type with bit 0 (`bob::animated_suit`).
- `People` draws whatever object is in the head slot (`bob::in_head_slot`)
  if its type has a suit that picks a mode (`bob::Mode::of_suit`).
  - It no longer checks for the standard bob's GUID.
  - Each suit's mesh is loaded the first time someone holds it
    (`Bob::load_suit`).
  - The colour is `bob::colour_in(mode, mood)`.
- `sims_render` no longer draws anything a person holds in slot 1 or in a
  slot past 4 (`SlotPoint::Skipped`). It used to draw such an object at
  its own position.
- The slot comes from the person's slot list entry that names the object,
  as the person draw reads it. The object's own slot word is used only
  when no entry names it.
- Slot 1 and the slots past 4 are skipped before the attach points are
  looked at. So a person with no posed body this frame, or a draw given
  no points, still leaves them out. What sits in the person's other
  slots stays at its own position, as before.
- An object whose holder words stop short of a holder can still be in a
  person's slot list. The held pass then places it by that list
  (`listed_in`), as the person draw walks the list and not the words. Where
  two lists name one object, the person being drawn wins, then the lowest
  ID.

## Pets

### What the original does

- **The same person.** Pets are people to the original (definition
  `+0x12 == 2`). They use the same draw (`0x459dc0`), the same held draw
  before the body (`0x459e45` → `0x45a131`), the same animator and the
  same attach-point binding (`0x5924a4`).
- **Every point binds.** The pet types' body strings, entries 4 to 7, are
  the same as people's: the head bone at the "Top" point, and the right
  hand at "Palm". The pet skeletons have the head, right-hand (a front
  paw) and pelvis bones, so every point binds. The first ADR guessed that
  pets have no right hand, which was wrong.
  - What a pet carries in slot 0 or 4 is drawn at its right front paw's
    "Palm".
  - Slot 2 is drawn at its pelvis, and slot 3 at its head top.
- **Their skeleton files.** The avatar registry finds skeletons by the
  name inside the file (`0x59f844`, reader `0x59f087`). The pets' files
  have an underscore before `skeleton` where people's have a hyphen.
- **Their empty head.** The pets' body strings name their head with the
  empty-slot word, the string at `0x6c0fd4`, which `0x40a5d8` writes over
  rows 3 and 31 to 33. The same word lists an empty suit slot
  (`0x58fc68`). No catalog has a suit by that name, so the slot binds
  nothing.

### What we do

- `appearance::load_skeleton` tries the hyphened file name, then the
  underscored one.
- `Outfit` and `Clothes::wear` skip the empty-slot word
  (`appearance::NO_APPEARANCE`), as they skip empty names.
- A character that Make New Character made, such as an adopted pet, has a
  new GUID that no catalog holds. It is looked up as its template, with
  the rows the simulation wrote put over the template's (`Cast::rows_of`,
  `house::written_rows`). The order is the one sims-vm reads them in
  (`read_character_row`): Make New Character's own rows win, then the
  outfit rows it and the outfit choice wrote (`World::made_outfits`), then
  the template's.
  - Row 0 names the skeleton, so a made child is built on the child
    skeleton (`Cast::residents_in`, also used by `residents_of`).
  - The rows also give the default outfit of a record that names no body,
    the suit's dress and the attach points.
  - The written rows are part of the body's cache key, so a changed coat
    loads the body again.
  - Outfit rows the outfit choice writes for a type that was not made are
    read the same way, as sims-vm reads them.

  Before, such a character was not drawn at all.
- Pet bodies now load, so pets are drawn through the person path, held
  objects first. Before this, no pet's body loaded. A pet was not drawn,
  and what it held was drawn at its own position.
- **Fallback, unchanged.**
  - A point that is unbound skips the slot (`0x591874`).
  - A person or pet with no body this frame gives no points, so what it
    holds stays at its own position.
  - Some template and NPC pet types name a head suit the install lacks.
    Those still fail to load and are reported as problems.
- Another person dressed with an empty slot (the Goth day's child at
  tick 7000 under the default frames) is now drawn without that part,
  instead of not at all.

## Tests

- `bob`: the suit picks the mode byte for byte, mode 2 swaps green and
  blue, mode 3 is yellow at any mood, and an animated type's suit is
  body string 2 before the comma.
- `appearance`: the empty-slot word names no part, and the skeleton file
  is tried hyphened, then underscored.
- `suit`: an empty hand slot keeps the head after it.
- `house`: a made type's rows are its template's, with its own written
  over them. Make New Character's rows win over the outfit rows.
- `scene`: slots 1, 5 and 9 of a person are skipped, along with what they
  hold, with or without the person's points. The slot list entry wins over
  the object's slot word. With holder words that stop short, the list
  still places the object, and still skips slot 1.
- `pets` (install, no names committed): every person type whose skeleton
  is in an underscored file loads that skeleton and binds all three
  points. Both pet skeletons have types whose whole body loads. On an
  install without the pack that brings pets, there must be none.

## Unverified

- **Not compared with captures.** No capture shows the date bob, the star
  bob or a pet carrying something.
  - The Goth day under the recording's frames does not make its cat in
    our run through tick 9000, so no live pet was drawn.
  - Pets' poses come from the simulation's animator, as people's do. When
    a pet has no saved animation, the fallback clip is the adult one.
- **Slot 1's bone name.** The world does not keep the bone name in slot
  1's record (`+0x20`), so `HEAD` is assumed.
- **Suits that pick no mode.** Such a graphic would draw as an ordinary
  posed model. No object in the install has one, and we draw nothing.
- **sims-game follow-ups** (not owned by this wave). Three checks still
  compare against the standard bob's GUID, so the date and star bobs miss
  them: the 3D pick (`pick.rs` `noted`), damage rectangles
  (`game/drawing.rs`) and the spin redraw (`game/ticking.rs`). Each should
  ask `People::is_bob`. The pet picker's `load_pet_body` can now use
  `Body::load`.
