# Decisions

Choices made while working toward `docs/roadmap.md`, newest first. Each entry
says what was decided, why, and what would reopen it. Entries marked
**unverified** wait on the comparison harness.

## 2026-11-15: A sound's volume and pan come from where its objects are on the screen

Kinds 1 (for a `kArgsIdVolPan` track) and 30 now set the track's
`extvol` and `pan` from every object recorded for it, as the original
does after starting it (0x573795 to 0x5737a4, 0x573d5d to 0x573d6e), in
`sims_audio::placement`. Each object is placed by 0x5730b6, and the sum
(0x57473b) takes the loudest volume and the volume-weighted pan, in
32-bit arithmetic that wraps.

The placement uses the screen when the track's `Spl` (register 57) is
not 0. `Spl` starts at 20, `spl_normal`, from the default bank for
registers 50 to 62 (0x6f6898, filled at 0x57c83a to 0x57c97b), and no
script in the install writes it, so every placed object's volume and pan
come from the screen: the pan from how far across it the object is, the
volume from the zoom (the far zoom takes 560 of 1024 away, the medium
280), less off the middle of the screen, and three fifths of that on a
floor the view does not show (`placement::project`). The game sends
each object's position (`sims_sim::Object::position`, the original's
+0xf8, +0xf4 and +0x110; `Snapshot::positions`) and its place on the
screen with the zoom, the screen's size and the floor shown
(`Snapshot::screen`, from `sims_game::sound::screen`), again whenever
the camera changes. The place on the screen is the original's (0x570e09):
the position rotated into the view about 1023, not the drawing's 1024,
then projected as the original's view does it (0x4a9dc7,
`sims_game::sound::fine_point`): the tile's corner, then each part of
the step across the tile cut toward zero on its own, in 16 bits. A point
can be a pixel off the drawing's (`project_fine`). The original also
raises the point, and slants the step, by the heights of the tile's
corners, which is left out (**unverified** on a lot that is not flat).
Without a place on the screen the placement is a fault.

The same bank corrects three defaults the player had wrong: register 39
(`BufSizeMult`) starts at 1 and 52 (`tempo`) at 120, and 40
(`fade_dest`) at 0, not 1. So a `seqgroup_kill` (0x57eef3), which fades
the holder without setting `fade_dest`, now fades it to 0, not 1.

What the original does with an object it cannot place is kept: an ID
that names no object, or an object with a first or second coordinate
of 0, leaves the previous object's values, and the first object's are
pan 0 and a volume a local held before: the address of the sound
manager's instance map (0x576235), on the heap. Here that address is
0x10000d4, **unverified**. It outweighs the other objects in the pan,
and both the pan and the volume come out where the 32-bit arithmetic
wraps them: with this address and a `vol` of 1024 the track plays at
212 of 1024, not at full. A `kArgsIdVolPan`
kind 1 with object 0 now starts nothing (0x57375b), where it started
the track; kind 30 records object 0 as an instance, where it skipped it.
The writes leave the comparison flags the pan's.

The volume and pan are worked out when the event comes, as the original
does, so `Player::handle` and `Player::play_from` now take the game's
`Sources`. A track's instances go in the original's multimap's order
(0x5761c2): the first recorded, then the others newest first. The
order the map's growth gives them (0x5762c7) is not ported; it only
matters for an object that cannot be placed.

Not ported: placing every track again on kind 33 (0x57447f), which the
original's game sends when the camera moves. The game here sends kind
33 every frame, and placing again would undo a fading `extvol` each
frame. Also not ported: the limit of 16 voices that kind 1 checks first
(0x574a87).

**Unverified:** nothing of this has been compared with the running game.

## 2026-11-15: The panel follows the lot; a portrait under the pointer shows a first name

The coordinator asked for the panel to be wired to live state: the
selected Sim's motives, the clock, the household's money and the
selected Sim. Most of it already was. `Hud::draw` lays the panel out
from `Snapshot::read(world)` every frame, and the window draws a frame
after every batch of ticks. What is new:

- **A test that it follows the lot.** The Goth test that queues Have A
  Snack from the fridge's pie now runs 850 ticks on. The panel's text
  has to show the world's clock and funds before and after. The funds
  show $2,542 and then $2,537, after Mortimer pays 5 at tick 820. The
  clock moves, and the needs panel's first meter has to be the selected
  Sim's hunger (motive 7) both times. `Game::panel()` gives the items
  last laid out.
- **The selected Sim.** The strip already lights the selected Sim's
  portrait (the template's selected cell). The original's live
  screen shows nothing else of the selected Sim's own. The icon in the
  top-left corner of the original's screenshots is the action queue
  (a bed, and a Sim's head for a social interaction), not the selected
  Sim's portrait. That queue is not drawn yet.
- **The name.** A person's first name is the first string, in US
  English, of the catalog strings (`CTSS`) that word 41 of the type's
  `OBJD` names. Mortimer's file has 2000 there, and `CTSS` 2000 starts
  with "Mortimer". Word 19 of the same file is 200, its body strings, so
  the count of words is right. `Faces` reads the name with the face.
  While the pointer is on a portrait, the panel draws that person's name
  in a box over the strip, above the portrait. The box takes no presses
  and does not cover the lot.
  **Unverified, and ours:** where and how the original shows a name.
  We believe it shows one on hovering a portrait, but no capture has
  been compared. The box's look (the needs panel's label color on
  black, font 8) is ours. Reopen when the harness can hover a portrait.
  The box is as tall as the font's line with 2 pixels above and below,
  so descenders stay inside it, and its bottom is 2 pixels above the
  strip. It is kept inside the frame: a name wider than the frame is cut
  short. The name goes when the pointer leaves the window. (Sol's review
  of the first version found the box overflowing at extreme rects,
  descenders below it, the box off the top of a short window, and the
  name staying after the pointer left.)

## 2026-11-15: A lit lamp's pool reaches the walls beside it; grass takes none

Straight walls now take lamp pools, from the wall tables in
`lightMap.dat`, as the original's wall pass draws them (read from its
code: 0x53db67 calling 0x4c79ca). A wall reads the lamp record of its
tile in the view: the tile whose low-x edge (a wall along view y,
direction 1) or low-y edge (along view x, direction 2) it stands on,
which is the side the camera sees (0x4aed23). Each of the record's two
lamps on that side of the wall (a view offset from the lamp to the tile
of at most 0 across it) gives an offset along the wall and a distance
out from it: for direction 1 `-dy` and `-dx`, for direction 2 `dx` and
`-dy` (0x4c78c5). A lamp counts if the tables hold an image for it: a
distance below 4 and an offset within `m - 1` of the middle, for the
distance's `m` of 5, 5, 4 or 3 (0x4b5c98). The image is direction 1's
table or direction 2's, at the zoom and distance, image `offset + m - 1`
(0x4b5c0c, 0x4bb595). It has a wall face's shape, as wide as the wall
sprite, and is read alongside the sprite with its read cursor starting
`1 << zoom` rows (2, 4 or 8, zoom counted from 1; 0x4bb4a2) before the
table, so its first row lies that many rows below the sprite's top and
the rows above take no pool. Each wall pixel
takes the drawn light byte plus each lamp's byte there, kept to 255, as
a floor does (0x4b7630 for one lamp, 0x4b7420 for two), and a room in
full light shows none. Diagonal walls take no pools (0x4c78c5 returns
none for them).

A corner's cap, the narrow slice of phantom wall face at an exposed wall
end, takes pools the same way (the cap pass reads the lamps at 0x53ca0a
and draws through the lit wall pass at 0x53ca41): the lamps of the tile
on its own seen side, along its own line, in the light of the wall it
takes its light from. Before, a lit wall's cap stayed in the room's
light, a dark seam at the wall's end (found in review; a lone wall with a
lamp beside it now lights its cap too).

**Checked.** The images' face lines up with our wall sprites' opaque
face exactly at all three zooms and both directions, with the first row
as many rows below the sprite's top as read. Compared with the original at the tick-570
world: far zoom at rotation 2 rose from 90.1% to 90.3% exact (non-green
87.9% to 88.2%); of the 1,250 pixels the pools change, 869 now match
exactly (1 did before) and the mean channel error fell from 26 to 1.1.
At rotation 1, walls up, 80.3% to 80.5% non-green; of 463 pixels
changed, 269 match (none did), mean error 16.8 to 1.7. Rotation 1 is
where turning the lamp record the other way with the view (0x4aedc2)
would show, so the turn is now checked for walls, and floors read the
same record the same way. The remaining changed pixels are mostly a
step off. Near and medium zoom show no lit wall and do not change.

**Grass** takes no pools: the lamp record is read (0x4aed23) only by
the floor draws (0x4adb48, 0x4af474), the wall pass's three wall calls
and a debug text, not by the grass.

**Not ported.** The wall blitter's second mode (0x4b780d, with
0x4c4a77 for unlit walls), which a caller's argument selects; our walls
draw by the first (0x4c4907). The original reads a record whose first
lamp is empty as having none even if its second is set (0x4aec97 counts,
then 0x4c78c5 reads from the first); ours never builds such a record.

## 2026-11-15: The game's speed and pause are the simulation's globals

The window paced the lot by itself: a tick every 50 ms at normal speed,
a third and a tenth of that faster, and no ticks while paused. The
simulation's globals were not touched. The original's code says
otherwise, so the window now does what its buttons do. Read from the
disassembly; none of it is checked against a run.

- The speed buttons call the setter (`0x46d697`) with 760 (normal,
  `0x4425a2`), 924 (fast, `0x46d264`) or 994 (ultra, `0x46d26a`). The
  setter clamps the speed to 0 to 1000 and keeps it in global 16. While
  global 18 is not negative, it keeps it there too. The time between
  two lot updates is 172 − speed × 0.001 × 171 ms, plus 0.5 and
  truncated (constants at `0x666a24`, `0x667720`, `0x66771c` and
  `0x667270`). That is 42, 14 and 2 ms. `sims_sim::speed` ports this,
  and `Lot::set_speed` takes it between ticks.
- The idle loop (`0x46d9d6`) runs an update each time a period has
  passed. The update (`0x46d345`) never reads the speed. The speed
  reaches the simulation only as globals 16 and 18. What reads them
  there was not looked for.
- Pausing (`0x46d813`) sets global 17, and running again (`0x46d83a`)
  clears it. The idle loop does not look at it, so a paused game goes
  on updating the lot. Each pass is then a partial one (`0x46d553`,
  already ported): only objects with scheduler flag 2 run,
  and the clock stands. `Lot::pause` and `Lot::unpause` do this. The
  window goes on stepping a paused lot. Strict mode on the Goths' house
  runs 200 paused ticks, and its clock stands through them.
- In live mode the speed buttons (`0x53a62a`, `0x53a6ad`, `0x53a728`)
  also run a paused game again. They ask the interface for it
  (`0x4cee4a`), which applies the speed first and then the unpause
  (`0x4ce5cc`, `0x46d83a`). `Game::set_speed` does the two the other
  way round. Both clear the same clocks with no tick between, so the
  order makes no difference. A button for the speed the game already
  has does not call the setter (`0x4ce588`), so the time owed carries
  on. Sol's review found both.
- **Unverified:**
  - The pause is refused while a field of the interface
    (`[0x6f2a58]+0x7c`) is not 0. Here it is never refused.
  - The setter and running again restart the idle loop's clocks
    (`+0xb8`, `+0xbc`), after which an update may be due at once. Here
    the time owed starts again from 0.
  - The original also caps how far it catches up (`0x4732de`). We keep
    our cap of 20 ticks a call.
  - Message 0xc2, which the setter and the pause send to the interface,
    and its effect on the sounds, are not ported.
- **Ported: the idle loop's own speed** (`World::idle_speed`, run by
  `Lot::step` before every update, in strict and play mode).
  - On a residential lot (`0x40745f`: zoning 1, `0x42b9e3`), before
    its updates, the idle loop asks whether anyone holds the game to the
    player's speed (`0x442f9b`, `World::speed_held`). A person in the
    registry's list holds it when all of these are true:
    - object data word 34 (`+0x8e`) is 0;
    - person data word 65 (`+0x60e`) is at most 1;
    - motive 11 (`+0x7b8`; motives start at `+0x78c`) is not below 0.0
      (`0x45edc5`, against the 0.0 at `0x66657c`; a NaN counts as
      below, as the x87 compare branches);
    - person data word 32 (`+0x5cc`) is 0.
  - While nobody holds it and global 18 is negative, the idle loop goes
    to 1000 (`0x46d790`): the setter makes the speed 1000, then global
    18 keeps the speed it was. While global 18 is not negative,
    `0x46d790` does nothing (`0x46d795`). So a speed the player sets
    meanwhile (the setter writes global 18 too) stands, not 1000.
  - While someone holds it and global 18 is not negative, the idle loop
    goes back (`0x46d7db`): the setter makes the speed global 18's,
    then global 18 becomes -1.
  - Both also restart the idle loop's clocks and send message 0x109 to
    the interface (1 going to 1000, 0 going back). That is not ported.
  - **Unverified:**
    - What word 34, word 65 and motive 11 mean. They presumably mark a
      Sim who is hidden (away at work or school) and one who is asleep.
      The full Goth day will exercise this when the household leaves or
      goes to bed.
    - Whether the original's speed changes on the Goths' house in the
      ticks we compare. The strict boot (House05, 1102 ticks) is
      unchanged by the port, so through it someone always holds the
      speed and global 18 stays -1.
  - `Lot::step` runs it before every update. The original's idle loop
    runs it on every pass, whether an update is due or not and paused
    or not, so `Lot::idle_speed` runs it between ticks too, and the
    window calls that on every pass of its own. A speed the player
    picks after the switch to 1000 then stands, as in the original. The
    check is idempotent: a second call in the same state does nothing.
  - The window follows the simulation's speed: ticks are owed at the
    period of global 16 (1 ms at 1000). Whenever the check calls the
    setter, even for the speed the game already has (going back to it),
    the time owed starts again and that pass runs no more ticks, as the
    setter restarts the clocks. The original may run an update at once
    then, if its second clock (`0x4732de`) is ready (`0x46da1b`); here
    the next tick waits a period of the new speed (**unverified**,
    window timing only). The panel's buttons still show the player's
    choice.
  - Sol's reviews found the guard that stood here before was wrong both
    ways, and that the check has to run between ticks as well. It failed where `0x46d790` keeps global 18's speed. It missed
    hidden and sleeping Sims, whose words it took to be unkept. Porting
    the whole head of the idle loop replaced it.
- Strict `sims boot` never calls the speed or pause hooks. The idle
  loop's speed never changes on the Goths through tick 1102, so the
  output is unchanged.
- **With the dialogs (#171).** While a dialog waits for the player, the
  window owes no time and does not run the idle loop's check: the lot
  takes nothing between ticks then (`Lot::idle_speed` is refused, as
  orders are). Once the dialog is answered, the time owed starts again.
  Whether the original's idle loop sets its speed while a dialog is up
  is **unverified**.

## 2026-11-15: The pie menu shows the Sim's head, drawn from in front

The original renders the selected Sim's head live into the middle of the
pie, in the blank face bitmap (`sims_ui::pie`, kPieFaceBkg, 201 by 201).
`sims-game` now does the same with `sims-people`'s public parts
(`People::head`). The Sim is posed as for the lot (`People::posed`). The
parts bound to the `HEAD` bone are skinned, which gives the head with its
hair, and glasses where a Sim wears them. The software rasterizer draws
them into a clear image the size of the face bitmap.

- The camera looks the face in the eye, 10 degrees from above. It is
  centred on the head and scaled so the head fills 80% of the face
  across or down. The light comes from the front and above. All of this
  is our choice. The original's camera, light, size and any animation
  of the head there (does it follow the pointer, or talk?) were not
  read, so the head is **not** compared with the original's.
- The face looks along the root's local +Z. This was found by drawing
  Mortimer: with +X his right side showed.
- The head is drawn again with every frame while the menu is open, from
  the pose of the moment. That poses everyone on the lot a second time
  for the frame. It only happens while a menu is open.
- A Sim who cannot be posed (not placed, no `HEAD` bone) gets no head.
  The face bitmap shows alone, and the problem is listed once among the
  people's problems.

## 2026-11-15: Manage Inventory walks an inventory; the advertisement record starts zeroed; scope 24 fields 0 and 1

Track 1's pie menus on the Goth house failed in check trees on these.
Listing every object's menu on the loaded house, 9 failed: 5 on Manage
Inventory operation 4, 1 on scope 12 and 3 on scope 24 field 0. Now 6
list. The other 3 (the phones) go on to scope 24 field 1, which is now
ported too, and then stop at Add/Change Action String's formatter
(0x44bf1a) on "Call Cab.../Go to Magic Town - $$Local:1", which is
track 1's.

**Manage Inventory operation 4** (0x455462, with 0x41f69c and 0x41f746)
finds the next token of a kind after the place in the caller's temp
`(flags >> 8) & 3`. With flag 0x1 it compares the GUID too; without it,
any GUID matches. Temp `(flags >> 5) & 3` takes the count and then temp
`(flags >> 11) & 3` the place. With none they take 0 and -1 and the
branch is false. Unlike operation 3, a token with a count of 0 is found.
The search starts at the place plus one as a word, so after 0x7fff it
wraps negative and finds nothing. The place written is the token's index
cut to a word, with no check (0x41f6d5 to 0x41f6e1; operation 3's
0x41f677 to 0x41f684 likewise, where this used to fail). A token at
index 65535 is written as place -1, and the branch, which tests the
place, is false. As for every operation, the owner's
inventory is made if it has none, and with GUID 0 the stack object's is
read first, whether or not the flag uses it. Setting the interface's
inventory flag on success is presentation.

**The advertisement record before the first copy.** The earlier entry
left it unread, failing loudly. It is now ported from the code: the
record's constructor (0x475e23) allocates its sixteen advertisements and
zeroes each one, and it runs from the static initializer 0x458271, which
is in the start-up table at 0x6c0154. Nothing else references the
record: 0x6e5100 appears only in that initializer, its destructor and
the pointer 0x6c437c. The pointer is read only by the three scopes and
the copy 0x4763c5. So until the first copy, scopes 12, 28 and 29 read 0,
and a copy of fewer than sixteen leaves zeros in the rest. In the World,
`advertisement_record` stays None until something writes it, and None
stands for those zeros, so the encoding and every hash are unchanged.
This holds for a game that has not run a lot since it started, as every
capture has.

**Scope 24 field 0** (0x425baa) is the ID (object +0xf0) of the first
person in the lot's list of people (lot +0x84, `World::people`) whose
definition (+0x11c) is the one the record is linked to (+8). It is 0 if
none matches. We compare GUIDs. When more than one definition has the
record's GUID, the reload can link a record it made to a replaced one,
which the World does not keep; that fails loudly.

**Scope 24 field 1** (0x425be2) is 1 if the selected family's members
(+0x12c, 0x4101f2) include the GUID of the record's linked definition,
which is the record's own (0x4572af); else 0. With no family selected it
is 0. The original reads the selected family's pointer (game +0x10).
Every setter of global 9 to a family's ID stores that family's pointer
with it (0x4f2218 with 0x4f222b, 0x4282b7 with 0x428317), and scripts
cannot write global 9. So a global 9 other than 0 names the selected
family. A global 9 of 0 does not:

- 0x415fc6 and 0x4164f6 clear it and select family 0 (0x415fd7,
  0x416509);
- 0x4278f6 clears it and leaves the pointer;
- the game's start and end set the pointer alone (0x415bd2, 0x415d21,
  0x415ecd).

The World keeps no pointer, so with global 9 at 0 the field fails loudly
(Sol's review found that reading it as no family was wrong when family 0
is selected). A global 9 that names no family is `Trap::Invalid`. A record of a GUID with no definition fails, as the
original would read through a null pointer.

**Verified:** the Goth house is byte-identical through tick 2881, hash
included (`eed688a0...`). The rest is checked against the code only: no
capture lists a menu.

## 2026-11-15: A click picks what the frame shows at the pixel

The game window picks from the per-pixel IDs the lot and its people
were drawn with (`Canvas::id_at`, previous entry), replacing its guess
from each object's tile and its own copy of the people's drawing loop
(`sims-game` now draws people with `draw_on_lot`, pixel for pixel as
before; the Goth test checks it against `sims-people`'s own call).
`sims_game::pick::at` takes, in the frame last drawn:

1. the person shown at the pixel, or the one shown nearest within two
   pixels (`REACH`, as before: people are thin);
2. else the object shown at the pixel itself, with no search around it.

So a person or object behind a wall or a bookcase takes no click meant
for what hides it, and a short object in front of a tall one no longer
takes clicks on the tall one's upper part. A click near a person still
picks the person over the object under the pointer, as before.

**A tile of a multi-tile object is picked as itself, not mapped to its
group's lead.** Each tile is its own object in the world, and the pie
menu is listed for the tile (`Lot::pie_menu`); the simulation then
sends a tile's interactions on to the tile that takes them, as the
original's `interaction_target` (0x435286) does (`menu::target`): the
first tile of the group whose definition word 8 is the clicked tile's
negated, when that is positive. Mapping every tile to the lead here
would skip that rule, which picks a tile other than the lead for some
groups. This is what the tile guess did too. **Unverified:** how the
original turns a click into an object (an ID buffer as here, or
otherwise), and whether it passes the tile or its lead to the pie
menu, are not read; compare the object the original lists at the same
pixel once the harness can click. Reopen if it maps to the lead.

## 2026-11-15: The player answers a dialog from the window

`sims_vm::boot::Lot` could only answer from a fixed list given at load,
so in the window the Goths' house had to say No to the Help System
before it opened. A lot can now wait instead (`Lot::wait_for_answers`).
When a tick shows a dialog with a choice and no answer is left, the
step does not fail. The lot goes back to how it was before the tick:
world, play-mode state, tick count and player. The dialog
(`Asking::shown`) and the world as the tick left it (`Asking::world`,
where the dialog's record is) are kept. Steps say `StepError::Asking`
and orders are refused until `Lot::answer` gives one of the dialog's
answers. The next step then runs the tick again from the start with
that answer queued. Until it has, orders are still refused: the lot is
as it was before the tick, and a listing or an order then would change
it where the original's never was (Sol's review found the window
listing a menu there). The simulation is deterministic, so the tick runs
as it would have with the answer listed at load. The test checks this:
the Goths' lot answered from the window gives the same world hash,
dialogs answered and sounds as `--answers 3`, tick for tick, through
tick 200.

Why the tick runs again rather than resuming: the dialog is shown in the
registry's pass at the end of the update (`0x4412d1`). That pass is
deep in the tick, and our interpreter cannot stop there and go on later.

- Strict boot never waits, so it is unchanged. `sims boot --answers 3`
  on the Goths' house to tick 1102 prints the same as main's, byte for
  byte. A lot that does not wait still halts at an unanswered dialog,
  as before.
- The cost is a copy of the world before every step of a waiting lot:
  0.1 to 0.4 ms on the Goths' house, measured with a step of 0.15 to
  0.6 ms. A tick at ultra speed has 5 ms here (`sims-game`'s pace).
- **Unverified:**
  - A sound the tick makes before the dialog is requested when the tick
    runs again, after the answer. The original presumably plays it
    before the dialog.
  - The original drew the house view many times while the Help System's
    dialog waited (`docs/roadmap.md`, the draw guard). We draw no frame
    of the simulation while waiting. A drawn frame writes animator state
    (`draw_frame`), so the frames are an input that may matter.
- The window (`sims-game`) always waits. A press on a button gives the
  answer the button stands for: 2, 3 or 4 for the first to the third
  (`Shown::answers`), and the tick runs again at once. While the dialog
  waits, the time owed to the lot is dropped, remainder and all, so the
  lot does not race to catch up after the answer.
  `--answers` still answers first. `--png` has nobody to answer, so for
  the Goths' house it still defaults to `3`.

## 2026-11-15: Pausing holds the sounds playing, as the original's sound manager does

The sound's pause and unpause, kinds 21 and 22, are ported from the
original's dispatcher (0x573c20, 0x573c44), as `SoundEvent::Pause` and
`SoundEvent::Unpause` and as HOT events of those kinds. The game sends them
from its mode handler (0x4ce470, through 0x57a7f2 and 0x57a8e3): pause when
it leaves live mode or pauses in it, unpause when it is back in live mode
and not paused.

A pause (0x57481e) marks every track that is running held (each track's
+0x1c8, through vtable +0x58, 0x57dc2e), except five: `ui_click`,
`MainMusicLoop` and the three speed buttons' sounds (0x5748a2). A held
track's update (0x57d7cd) does nothing, so its script waits and does not
fade, and its buffer stops where it is. Here its threads are skipped and
its notes neither advance nor sound. The pause also writes 0 to
`main_duckpri` (0x57d361), which here drops every thread's ducking
request. In music mode 0 the current ambience player is paused too
(vtable +0x7c, 0x5773af): its update returns at once (0x577af9) until it
is unpaused, while its cells' sounds play on. Starting a track
(0x57d960) clears its hold, so a sound started while paused plays (the
buy mode's clicks do); a held track whose script still runs is not
started again but let go, as an unpause lets it go (0x57d9f1 to
0x57da12), and needs no free thread. Before that check, the start
points the track's `target` back at itself (0x57d9c3) and writes its
argument registers 1 to 3 (0x57d9cb to 0x57d9eb) whatever its argument
kind, so a resumed script sees the new arguments; a new thread here
now gets all three too, where it had only `arg1`, and only for
`kArgsIdVolPan` tracks. A fresh start then writes `isplaying` (register
46) 1 (0x57dbc5), so its script begins with positive comparison flags
whatever its third argument, and ends by holding the track (0x57dbd6).
Kind 1 always lets it go; kind 30 (0x573d14) lets it go only when the
sound is not paused (0x573d71), so a television or radio turned on
during a pause is silent until the unpause. An unpause (0x574904) lets every track go
(0x57cd2c, vtable +0x5c on each) and unpauses the current ambience
player, whatever the music mode. Letting a track go also sets its wait
deadline (+0x198, which `wait` sets at 0x57e453 and the update reads at
0x57d8b3) to the manager's time, so every running script's wait ends at
the next update: a thread waiting a second when the game pauses goes on
as soon as it is unpaused, and an unpause without a pause cuts every
wait short too. Here every thread's wake is set to now.

Not ported: the day and night loop (0x574978), which the pause stops and
the unpause restarts, and the music the unpause restarts (0x575155); this
player has neither. Where the original keeps one object per track, a
track here can have several threads, and the hold is per track, so
starting a held track lets every thread of it go on.

**Unverified:** nothing of this has been compared with the running game.
Reopen when a capture with sound can be compared.

## 2026-11-15: The base layer kept; a person takes itself off; strict mode to tick 3432

**Unverified.** No capture covers any of it: everything here is from
tick 2837 on, and the Goth captures end at update 1000. Strict mode now
runs through tick 3432 (08:25:08, hash 359095d7…; fbb3d668… in format
25) and stops at tick 3433.

**Hashes.** Snapshot format 25 appends `World::base_layers`, and format
26 then `World::translated_bones`, so every pinned hash changes. The
format-25 encoding is a prefix of the format-26 one, and `boot.rs` and
`resize.rs` pin both (`format_25_bytes`): every format-25 hash this
entry first pinned (ticks 33, 1103, 2881 and 3432) still holds. The
format-24 encoding is a prefix of the format-25 one, and `boot.rs` pins
both (`format_24_bytes`): ticks 33 and 1103 keep
their format-24 hashes, and the state at tick 2836 is byte for byte the
one the previous commit (1bacd2d3) reaches (format 24, f74e31af…).
At tick 2881 the format-24 hash changes from eed688a0… to cdd5b72c…,
which is the head fix below; ticks up to 2836 are unchanged. Every
summary line of `sims boot … --ticks 2881` is unchanged. The K=1000
pins in `resize.rs` hold.

**The head looks at an object with its coordinates the right way
round** (0x59058f, 0x454f5c). The measurement passes native +0xf4 first,
which is our second lot coordinate, so the target is [3/16 ·
position[1], 0, 3/16 · position[0]]. #156 had them swapped. Track 4
found this and a review agreed.

**The base layer and the pose** (stop at tick 2882, 0x5992b3). The base
slot's playback is kept in `World::base_layers` and updated with the
skin's other layers in the animator's tail, timed events included. The
pose samples the layers by priority (0x5991fd): the base layer (-10)
first, then the foreground and the carry layer in the skin's order.
Each layer at weight 1 or more replaces what the bones held:

- A layer with nothing bound, no frames or no weight samples nothing.
- One below weight 1 blends (0x597c4c), which fails.
- The skin never puts the bones back at rest. A rotation on the way to
  a measured bone that no layer playing now writes is what an earlier
  layer left, which is not kept, so that fails.
- A translation is kept too: a motion with no positions leaves it as
  it was (0x598226, 0x598450). `World::translated_bones` (snapshot
  format 26) records, per person, every bone whose translation a layer
  the skin's update could sample has written since the person was made
  or loaded. A bone on the way to a measured bone that no layer playing
  now moves fails if it is in that record, and is at rest otherwise. The
  record counts every layer that could sample, without the update's
  "pose" argument (0x59820a) or the sample count, so it may hold more
  bones than the original moved; that only makes more cases fail. That
  a skin starts at rest at load is unverified. Of 7,950 clips, 7,947
  move ROOT and 7,577 PELVIS; SPINE is moved by 32.

A posture whose clip has no name starts nothing and leaves the slot as
it was (0x5904af, from the disassembly; retried next tick). Animate Sim
in the base slot now plays clips with timed events.

**Generic Sims Call 12** (stop at tick 2930, 0x44fd4b). Temp 0 becomes
minus the sum of the stack object's tile coordinates, turned by the
house view's rotation (0x434f58, the table 0x407704 builds), as a
16-bit word. A coordinate outside 0 to 63 at a rotation other than 0
fails, since the original reads outside its table.

The view's rotation (the scene at 0x6e5528, +0x84, passed at 0x44fdb8)
is the player's, not the lot's, but the simulation reads it, so it is an
input of `Lot` like an order: `Lot::set_view_rotation`, between steps.
Every interpreter `Lot` builds (load, step, orders, pie menu, selection)
passes it.
The game window sets it whenever its camera turns, from the panel or the
keys. **A recording of play must log each change** with the tick it came
before. The load starts it at the camera direction the house file saved
(HOUS, `boot::opening_rotation`), as the render track opens the view.
That the original's load turns the scene to it is not traced (its HOUS
read goes through 0x4b4263 and 0x4b42ba). The Goth and Newbie houses
both saved 0, and the comparison harness saw the Goth house open at
rotation 0, so strict boots of both run at 0. Which way the panel's two
buttons turn the original's scene is unverified.

**Generic Sims Call 2** (stop at tick 3356, 0x44fb51). It names the
running action's icon. #158 ported the same routine on main with the
same meaning, so the rebase keeps main's.

**A person takes itself off the lot in its own tick** (stop at tick
3356; Find Location mode 1 with the person as target). 0x433125
refreshes the person's animator before the registry hears of it
(0x4332e0, virtual function 0x54 with 1, 0x58da12), but only if animator
byte +0x109 is set. That byte is the house view's last draw test
(0x590b8b), which is presentation and is not kept. The refresh is a
small animator tick: the clock, the blend slot, the skin forced, the
place, and the head's step, measurement and restoration. Instead of
choosing a branch, both must agree. This is exactly what is checked:

1. At the take-off, the refresh is worked out on a copy of the lot as
   the take-off leaves it (after the holder's function 9 and the
   portal's tiles, before the registry, which does nothing for a
   person). The copy, with the person's animator, base layer and record
   of moved bones (`World::translated_bones`) put back, must encode byte
   for byte as the lot does, and the refresh must play no sound into the
   sound list. So the refresh writes nothing but those three, judged on
   the state it would really have read. A running head countdown (word 45) fails here, and so does a
   timed event that writes a hand or the censor word to a new value.
   The sound list does not hear the sounds of timed events, which are
   not modeled. So a timed event due in the refresh with a sound key
   ("sound", "selectedsound", "deselectedsound", "footstep") fails
   too: for a selected person "selectedsound" plays (0x59158c, 0x59175e,
   0x57a54c), and when the original plays nothing is not worked out.
   The lot goes on without the refresh.
2. Until the animator's tick, the person's thread may run only Sleep,
   Expression, Set Motive Change, Set To Next, and Change Suit with
   operand 1 set to 1. Anything else fails, naming 0x58da12. That these
   read no animator and no base layer is from their ports, not proven
   for the original.
3. At the animator's tick, the animator, its base layer and the record
   of moved bones must be those the take-off left (compared field by
   field).
4. The tick from the refreshed three, on the lot as it is then, must
   reach the same encoded world, and append the same sounds, as the
   tick alone.

What this does not cover: anything the refresh does that the port
leaves out (the draw state, the face, the pose, the head slot's
redraw). In the tick itself the sounds of timed events are still not
modeled, in both branches.

The tick alone is kept. The refresh then redraws what the head slot
holds (0x58dae5 to 0x58dc55): the container entry at begin + 0x48, our
slot 2, read when the person has more than two entries. For an ordinary
object (vtable 0x666d34) every write lands on its graphic ([obj+0x120],
from vtable +0x2c, 0x4215c3): the head bone's transform (animator +0xac,
0x5990e0, times animator +0xb0) through graphic +0xac, +0x64 and +0xb8,
and graphic +0x100 = the person's motive 3 as an integer (0x45d117 reads
+0x78c + 3 * 4; 0x636128 is the C runtime's `_ftol`, not a clock or a
random draw). Its +0x78 call is 0x51d372, `ret 4`. The animator tick's
tail (0x58f908 to 0x58fa7f) makes the same redraw every tick, which the
port already leaves out as presentation, so the take-off's redraw is
left out too; that is unverified. A person or a tile of a group in the
head slot fails, since their classes' functions (0x58dc08 to 0x58dc52)
are not read; this is checked before and after the take-off's
callbacks. A held hand (slot 1) fails too, though the refresh does not
read it, which can only add stops. A multi-tile person fails. The draw
state (0x434101) and the face (0x58faf6) are taken as presentation, as
in the tick; that is unverified. At tick 3356 person 21's head slot
holds its plumb bob (object 383, GUID 0x2F726FFF), an ordinary object;
person 21 then passes all four checks.

**Go To Routing Slot's interrupt tests** (stop at tick 3370):

- the door step (0x45afd0) and the asking step (0x45b475) fail with
  result 4, as step 1 does;
- the walk under way (0x45b2b7) takes result 4 to step 11, which stops
  the animator, plays `end_route` and fails (0x45b3c2).

These are on track 4's list; strict mode on the Goth house needed them,
so they are ported here. Only the door step has a synthetic test. The
other two use the same interrupt check and are tested only by the boot.

**Where it stops.** In strict mode, at tick 3433. Object 334 (tree
0x200d node 5) snaps a multi-tile object to a place where it does not
fit, and which object its group's test leaves as collided with is not
ported (0x454e82). The snack order (play mode) now runs without a caught
error until step 3603. There, object 354 (tree 0x1022 node 29) uses
Create New Object Instance mode 6 with flag 0x4, which is not ported.
With this and #158, strict mode on the Newbie house (`House07.iff`) runs
through tick 1472 (no recording to compare); at tick 1473 object 105
(tree 0x1017 node 0) gives up a route and needs the route-failure
feedback tree (0x45af2a), which is not ported.

## 2026-11-15: The canvas notes which object or person drew each pixel

For the game window's clicks, `sims_render::Canvas` keeps an ID per
pixel beside its colour and depth. Whatever draws sets a pen
(`Canvas::set_id`); every pixel the canvas takes after that, by its
usual rule (passing the depth test, not fainter than `FAINT`, blended
or not), takes the pen's ID. `Canvas::id_at(x, y)` gives it back:
`Option<u16>`, the key in `World::objects`. `sims_render::draw` clears
every ID and gives object sprites their object's ID and terrain,
floors, walls, trims and caps none. `sims_people::People::draw_on_lot`
(and `People::draw` into any `Target` that keeps IDs, through the new
`Target::set_id`, a no-op by default) gives each person's pixels the
person's object ID. So after the lot and its people are drawn, `id_at`
names what is shown at a pixel; an object or person hidden behind a
wall is named nowhere, and a pixel a soft edge blended over names the
edge's owner. Each tile of a multi-tile object is its own object and
names itself; finding its lead object is the caller's. Nothing changes
what is drawn: the Goth house digests are unchanged.

For `sims-game` (track 1): its people loop (`Marked`, `shown` in
`src/people.rs`) can set the canvas's pen per person (or call
`draw_on_lot`) and read `Canvas::id_at`, and `src/pick.rs` can pick
objects by it instead of by tile. Whether to search nearby pixels when
the pixel itself names nobody (`REACH` today) is the game's choice.

How the original resolves a click to an object or a Sim (by an ID
buffer, as here, or otherwise) is not read; this is ours, and what it
names should be compared with the original's choice of object at the
same pixel once the harness can click.

## 2026-11-15: A lit lamp throws a pool of light on the floor around it

The bay's floor was lit in graded pools the renderer did not draw. They
come from the lamps, not the windows: in the original a lit lamp marks
the tiles around it, and each floor tile it reaches adds a pool map from
`GameData/lightMap.dat` to its light. `sims_render::pools` reads the
maps, `Lot::lamps` keeps each tile's lamps, and the floor draw adds the
pools. A lit lamp's own sprite is drawn in full light.

**Read from the code.**

- The marks (0x467e05, 0x4b4e8b, 0x4b5037). As the room update counts
  an object, one that is a light source (data word 40 flag 0x400), gives
  light (word 51 not 0) and has word 16 set to 1 takes its marks off and
  marks again: every tile within 4 tiles each way (12 / 3, rounded), on
  the lot's inner tiles and in the lamp tile's room (for a split tile,
  either half's), takes the offset from the lamp. A tile keeps two: a
  third lamp nearer than the first replaces the first (which is lost,
  not moved down), else one nearer than the second replaces it.
- The pools (0x4c811f, 0x4c8049, 0x4b6c17, 0x4b61b2). A floor tile whose
  room light is not 1 (after the 0.5 to 1 clamp) draws with each lamp
  within 3 tiles each way (the map's reach, 0x4a3805), the view-rotated
  offset choosing where in the map the tile's image lies (0x4a3813). A
  pixel's light byte is `fraction * 255 + 0.5`, truncated, plus each
  lamp's map byte, capped at 255; its top five bits pick the level as
  usual, so the brightest pool is level 31, never full light.
- The file (0x4b5700). Four light fields, then the wall tables, then a
  pool map per zoom. The light radius of 12 gives the field and table
  sizes (0x49bdf6, 0x49be1f, 0x4bb7c5), and the parts then add up to the
  file's 2,271,104 bytes exactly. That is how the maps' sizes were
  found: 7 tiles across and half that high at each zoom.
- A light source other than a window (type 8) with light (word 51) draws
  in full light (0x437f7d, 0x4b455f).

**Compared** with the world at the screenshots' time (a snapshot at tick
570, 6:49): near zoom rose from 85.2% to 86.1% exact (non-green 90.9% to
92.0%), and far zoom at rotation 2 from 89.9% to 90.1%. Far zoom at
rotation 1 and medium zoom do not change: no pool shows in them. The
floors in the bay now match; what differs there is the edge-on wall
(fixed in #159) and the shrub's shadow.

**Not verified.** The original builds its marks up as it counts rooms,
in its own order, and only takes a lamp's marks off when it next counts
it dark; we work them out afresh, lamps in object order. They could
differ where three or more lamps reach a tile or a lamp went dark; no
screenshot shows either. Which way the record turns with the view
(0x4aedc2) is not read: we turn each offset as the camera turns the
lot, so a pool stays centred on its lamp. Turned the other way, a tile
off the lamp's axes would show its gradient reversed; rotation 2 cannot
tell the two apart, and no screenshot at rotation 1 or 3 shows a pool. When
the file is missing the original computes the maps (0x4bb5bc); we draw
no pools. Data word 16's meaning is not known.

**Not ported.** Pools on walls (0x4c78c5: the file's wall tables, by
the lamp's distance from the wall and its offset along it, blitted by
0x4b7630 and 0x4b780d), on grass, and the full light a draw group's
sprite gets when a word of its record is 7 (0x437adb).

## 2026-11-15: From house version 63 the floors and walls are read whole from Arry 11, 12, 111 and 112

The lot array loader (0x478599) tests the house's version once per level
(0x478617): from 63 on, it reads the lot's floor array from Arry chunk
level base + 11 and its wall array from base + 12 (0x47861d to
0x478652), and skips the widening of chunks 1 and 2 that older versions
need. The read (0x431005) replaces the whole array, taking the chunk's
size and cell size, as for every other array. So a version-63 house's
floors and walls are 2-byte and 14-byte cells as the lot holds them,
every cell of them, not just those inside the lot's size. `World::read_arrays`
now does that (`load::WIDE_SURFACES`). The loader's other version tests
are all below 53, so this covers every later version too (the expansion
lots are 69 to 73).

Every shipped house of version 63 or later has these arrays at 64 by 64,
with 2-byte floor and 14-byte wall cells, and the port requires that: any
other size or cell size fails here. The original's read takes any size
(its resize, 0x4312ff to 0x431388, has no 64-by-64 requirement), and what
follows with another shape is not known. Arrays 1, 2, 101 and 102 left in such a file are not read, as in
the original.

The floor and wall maps already followed the version (chunk 1 and word IDs
from 63, `House::load`).

**Verified:** the Goth house (version 62) is byte-identical through tick
2881, hash included (`eed688a0...`). Booted together with the records
change, UserData3's house 2 (version 63) gets through the arrays and the
renumbering and stops at the household selection (it has no family).
**Unverified:** a version-63 lot's arrays against a capture.

Next for the template houses: UserData2's houses 1 and 9 have floor and
wall maps with entries, and resolving those to the game's loaded patterns
is not ported (`World::renumber_surfaces`).

## 2026-11-15: Sound scripts ask about an object as the original answers

`getsrcdatafield` passes the values of both its source registers, the
object's ID and a field, to the game's callback (0x57f2e8 calls 0x571127).
We had passed the thread's own object as well and answered 0; the
`Sources` trait now takes the two values and may refuse. A field of 21 or
more is refused, as the original refuses it: the destination keeps its
value and "Illegal source parameter request" is a fault. Any other field
gets the original's object query (0x570e61), listed in
`sims_audio::sources`: field 0 is the voice (1000 for person data word 74
bits 2 or 3, 2 for a child, else the gender word 65), 1 the age, 2, 3,
5 to 7, 11, 12 and 15 to 20 person data words over 10 (skills and
personality), 8 to 10 zero, 14 motive 3 over 10, 4 the hour (global
0: the record at `[0x6e48bc]+0x1c` is the globals block, since its +0x24
is global 10, the lot), 13 the size in tiles of the object's room, or 100.

The original asks the live simulation; the sound here runs on its own
thread. So `sims_game::sound::sources` reads every object's answers from
the world into a `Snapshot` each time the window runs the lot, and sends
it to the mixer before that run's sound events when it changed. A script
can hear a later state than the one its sound was asked in (the window
may run several ticks at once) or an earlier one (the mixer's timing);
nothing in the sound is known to depend on the difference. It only reads the world, so strict output is unchanged.
On the Goth house the answers are what the save holds: Cassandra 2,
Bella 1 and Mortimer 0 for the voice, and their personalities.

**Unverified:** none of the answers has been compared with the running
game, and the meanings of the fields beyond what the code reads (voice,
gender) are guesses from the person data words. Reopen when a capture
of the sound manager's queries can be compared.

## 2026-11-15: The reload makes a record for each person type without one; UserData2 to 8 load

The houses of UserData2 to UserData8 failed before any tick: "the person
type 0xA5C5AEE7 has no neighborhood record". It is not a loading bug. We
read the right files (each `UserDataN/Neighborhood.iff`, with
`UserDataN/Characters` registered), but those are the 2000 templates,
never opened by the Complete Collection: 21 to 23 records, against 231
person types of the expansion packs and downloads (0xA5C5AEE7 is
`Downloads/NPC_JonBonJovi`) that have none. The install's `UserData` file
was saved by the original after it had made them, and has 463.

What the reload (0x424806) does to the records is now ported, from the
original's code:

- Reading a record links it to the definition its GUID resolves to
  (record +8, 0x423ef2, the registry's map 0x43ac10). The reload deletes
  each record linked to none (0x424ad8, 0x4259a2: a signal 0xfb, the
  record's inventory emptied of kinds 0 to 9, its slot cleared, the first
  row aimed at it removed from every record). No shipped neighborhood has
  one, so that **fails loudly** instead of being ported. 0xfb's
  subscribers were not checked.
- After the families are read and sorted, it walks the registry's
  vector (registry +0x20, 0x43ac3a), which keeps every definition in
  registration order, duplicates included. Earlier in the reload
  (0x43c228, at 0x424a8b) the neighborhood's characters are registered
  again, which appends them to the vector after every other definition.
  Our catalog has them before `Downloads`, so `registry` moves the
  `Characters` mount's definitions to the end (Sol's review found this:
  in a neighborhood missing records for Jon Bon Jovi and User00122, slots
  18 and 414 would have gone the wrong way round). For each of OBJD type 2,
  0x200, 0x400, 0x800 or 0x1000 (OBJD +0x12) that no record is linked
  to, it makes one (0x4257a5 with slot -1, 0x4237a1). The new record
  goes in the **last** empty slot, or in a new one at the end. Its ID is
  the slot's place plus one. It has the definition's GUID (0x4572af),
  revision 0 and every data word 0, with no rows and no inventory. A
  definition that a later one of its GUID replaced gets a record too,
  since records link to the replacement. The types beyond 2 matter: the
  install has 39 such definitions (36 of type 0x400), which the old
  check missed.
- Families drop members before the records are made, so a member whose
  record is made then stays dropped. Word 61 is rebuilt after.
- The new record's friend count and its out-of-date flag (+0x20, +0x24)
  are not set by the constructor. The original can read them before a
  relationship write sets the flag: scope 24 field 5 (0x425c19,
  0x427148) returns the count as it is when the flag byte is 0. Here
  they are 0 and set, as for a record read from the file. That field
  fails loudly, so nothing reads them.
- The original reads the records only when the file has an NGBH chunk
  (0x424a90); without one it makes them all. Ours reads them either way,
  and every shipped neighborhood has one. The templates' NGBH is an old
  version, which the reader (0x4254f6) takes; below version 0x45 it
  sets the pending-patch flag (+0x1aa).
- Its name (+0x28) is presentation. So is the list of deleted people's
  names (+0x168).

`read_neighbors` now takes the registry
(`sims_sim::neighbors::registry`, from the index and the catalog)
instead of the person GUIDs.

Right after the reload, the house load (0x428041, 0x428071) deletes each
family other than family 0 with no members (0x425efb). No shipped
neighborhood has one, so `World::drop_empty_families` **fails loudly** for
one.

**Verified:** the Goth house's strict output is byte-identical through
tick 2881, hash included (`eed688a0...`, default frames, and with
`--frames none`). The base neighborhood already has a record for every
registered definition, so nothing is made there.

Checked against a capture of the original's registry in the base
neighborhood (`newbie-startup-20261021/live-catalog.json`): with the
characters moved to the end, its 463 definitions of these five types
come in the same order as ours. The whole registry differs elsewhere
(seven definitions we register that it does not, and an order that
parts at entry 646), which the reload does not read.

**Unverified:** the records made for a template neighborhood against a
capture.

What the template neighborhoods still need before they match the
original: opening a template neighborhood imports the families in its
`Patch` folder (`*.FAM`, 0x42bca4, called from 0x42c0ff behind expansion
bit 0x20 of `[0x6e48bc]+0x66` and the neighborhood's pending flag +0x1aa).
That runs from the neighborhood view with a dialog and writes character
files, before any house load. The expansion families of the base file
(2000, 3000 to 3009, 5000, 5500, 6000) come from somewhere else again.
Neither is ported. A recording of a UserData2 to 8 house should start from
a neighborhood the original has already opened and saved, with those
files copied into the cache.

Where those houses stop now:
- All ten of UserData2's base houses are version 63. So are houses 2, 3,
  5, 6, 7 and 10 of each of UserData3 to 8; the rest are 62. Version 63
  fails at the arrays: floors and walls are Arry 11, 12,
  111 and 112 there, which is not ported.
- The version-62 houses of UserData3 to 8 have no family and stop at the
  household selection.
- The only family in UserData2 to 8 lives in UserData2's house 9, which
  is version 63.
- The expansion lots (versions 69 to 73) stop at the object read. Their
  people's definitions are in no registered file.

## 2026-11-15: The game draws its pie menu with `sims_ui::pie`

The window's pie menu is now `sims_ui::pie::Pie` (#157), the port of the
original's menu window and object picker, in place of the plain list
`sims-game` drew. The game hands it the entry names in the listing's
order and keeps it while the menu is open; the pie groups a name with a
`/` into a submenu and shows at most 12 entries, and a choice is the
entry's index in the listing, which the game queues as before. A press
on a submenu's entry, or back out of one, keeps the menu open; one
elsewhere closes it. The pie is laid out as it opens and again after
each press that keeps it, so a second press finds what the first
opened. As `sims_ui::pie` says the original does, the window moves the
pointer to the middle of a submenu that opens, and of a menu that had
to move to stay on the screen; where the platform refuses to move the
pointer, it stays.

**Verified:** unit tests choose an entry, open a submenu and choose
from it, press twice with no drawing between, move the pointer for a
menu moved on the screen, and close the menu; on the Goth house a test
clicks the fridge, finds Have A Snack on its pie and presses it, and
Have A Snack (its menu number on the fridge) is queued for Mortimer.
Moving the real pointer is not tested; no test opens a window. The placement itself is `sims-ui`'s, checked there
against the running original.

**Not verified or not done:** the middle of the pie shows the face
bitmap, which is all colour key, without the selected Sim's head; the
original renders the head into it live, which needs `sims-people`. The
game takes a press when the button goes down and never draws a label
in the held colour (`pressed` is always false); which of down or up the
original's picker answers on is not checked here. Reopen when a
screenshot of an open pie disagrees, or when the head is rendered.

## 2026-11-15: Deselecting a Sim is ported but for its animator's refresh; play mode switches Sims

Selecting a Sim (0x445449) now ports the deselection of the Sim selected
before, read from the original's code (0x445449, and 0x44664c from
0x4468c3 for a Sim without the mark), with the old prototype as a guide
(legacy/crates/sims-vm/src/selection.rs and selection_indicators.rs).
The old Sim's mark is taken off. With nobody selected, the first
indicator is taken off the lot (its virtual function 0x50 with 1, as
`Interpreter::take_off` takes an indicator), the Sim's animator is
refreshed, and the second indicator is taken off too. A Sim with person
word 81 at 10 or more would then take a Superstar star marker
(0x446c15); that fails loudly, whatever the expansion mask, as the
placement of a person from outside does. The redraw (0x434101 with 0)
is taken to be presentation (**unverified**). Then the new Sim is
selected as from nobody, as before.

The animator's refresh (virtual function 0x54 with 0, 0x58da12) is not
ported. With 0 it always runs: it reads the clock again, which between
ticks has moved on from the animator's, brings the skin up to date,
places the animator, and runs the head's step, which can count person
word 45 down and write word 42. So in strict mode (`Interpreter::select`,
`Lot::select`) deselecting a Sim while the lot has indicators fails
loudly there, and a selection that fails changes nothing. Play mode
(`Lot::select_play`, `Refresh::Skip`) leaves the refresh out, so the
deselected Sim's animator, head and words do not move as the original's
would; such a lot is not one to compare with the original. The game
window selects with `Lot::select_play` in play mode and `Lot::select`
in strict mode, where switching Sims therefore fails with a note.
`sims-game` carries out the panel's `Command::Select` this way, and the
panel, the motives shown and the pie menus follow the new Sim. Which people the original's
portrait press lets the player select, and whether it does more than
0x445449 (the camera following the Sim, for one), is not read: the
panel shows only the household's portraits.

**Unverified**: no recording selects a Sim. Strict `sims boot` of the
Goth house is unchanged (1102 ticks, output identical); a removal of the
selected Sim (0x44123f) reaches the new path, which still fails there
(now on the refresh rather than on "deselecting is not ported"), and the
Goth run removes no selected Sim. Reopen when 0x58da12 is ported, or
when a recording of play selects a Sim.

## 2026-11-15: The game lists the pie menu on the lot when the pointer reaches an object

When the pointer first reaches an object or a person on the lot, the
game lists the selected Sim's pie menu on it with `Lot::pie_menu`
(0x48d392), as the original lists it then (0x48c834); the listing keeps
what its check trees change, and its sounds are played with the steps'.
An object reaching a pointer that stands still counts too: each frame
drawn, the object under the pointer is looked up again. Nothing is
listed over the panel, a dialog or a menu, or once the lot has stopped.
A click opens that listing (or lists now, when the pointer did not move
onto the object first). Only the entries whose check passes are shown,
in the original's order. An entry a check tree named with Add/Change
Action String (#140, as the fridge's meals are named) shows that name as
it is; any other, or one named with an empty string, is named by the
TTAs strings with the ID of the TTAB the simulation reads, by its menu
number, with "*" before it when the entry has flag 0x80, as the
original's getter does (0x41f167). Choosing one queues the very action
offered (`Lot::pick`). Where the simulation has not ported something the
listing needs (a trap of an unsupported kind, scope or primitive: an object
offers nothing; a version 0 table), the lot is unchanged, the menu shows every entry of the table
without checks, debug entries hidden, queued by menu number
(`Lot::queue_interaction`), and the note says why. Any other failure
shows no menu. The simulation reports every fault of a check tree as
unsupported and keeps the fault inside only as text, so for those the
game reads the inner trap from the text: a check tree taking its error
branch or running out of instructions shows no menu.

This runs menu checks on the shared simulation, where
`docs/vision.md` ("Other rules") runs them on a throwaway copy per
player. The original does it on the lot in single player, and #138 made
`Lot` do the same; the vision's rule is for multiplayer and is to be
settled there. Reopen when multiplayer lists menus, or when a recording
shows a listing at another moment.

## 2026-11-15: The ambience follows the outdoor share of the screen, as the original works it out

The camera value that moves the ambient score's row is ported from the
original's code (0x570c3e, which its sound manager runs each frame at
0x574161): `sims_audio::ambience::view`. It walks the screen in rows half
a tile high (`height / (4 << zoom)` rows of `width / (16 << zoom)` tiles),
from the view tile under the top left pixel, and counts the tiles on the
floor shown whose tile flag 4 is set, "outside every enclosure" (read by
0x489260, which first turns the view tile back by the camera's rotation).
The value is `outside / counted * 100.0` on the FPU, cut toward zero; with
nothing counted the division gives NaN, which `__ftol` makes 0. Below 30
the player takes it as 0, as before.

`sims_sim::World::outside(level, tile)` is the read: the tile's flag 4
in arrays 8 and 108, as the last room rebuild left them. It only reads,
so the world's hash and strict mode's output are unchanged. `sims-game`
sends `Game::ambience_view` to the mixer with the hour and speed, in
place of the fixed 50 it used. On the Goth house at 800 by 600 it gives
about 92 at the far zoom, 79 at the medium zoom and 26 to 37 near.

**Unverified.** Which tiles count: the walk takes a view column and row
from one past the lot's bounds (lot +0x70 to +0x7c) to one short. The
bounds are taken as the 64-tile arrays' 0 and 63 (tiles 1 to 62), which
is the same in every rotation; `sims_vm`'s on-lot check makes the same
compare with the same unverified values. The corner tile is asked of the
original's camera (its function 4, with 1) and taken here as the ground
level's fine position under the pixel; and the screen size is the house
view's. None of the values has been compared with the running game.
Reopen when a run with sound can be compared, or when the lot's bounds
are read.

## 2026-11-15: An edge-on diagonal wall stands 300 nearer, anchored halfway along

At the bay's two ends the original shows the outside shingles; we showed
the inside wallpaper at the left end. It is not a side the original
picks. What the original shows there is mostly the straight bay walls'
side faces, which come out from behind the diagonal; the edge-on
diagonal (frame 3) is hidden behind them. We drew the diagonal too
near, so it covered them.

**The side (read, not ported).** Each run in the original stores two
patterns per entry: ascending runs [p2, p3] and descending runs
[p3, p2], slot 0 (0x53f6c1, 0x53f3e9). When the view rotates, the run is
turned (0x53d93a). It is reversed when its end sorts before its start
(0x44054c), which swaps every entry's sides, and the slot comes from the
table at 0x53db21. So the original's edge-on diagonal shows the side
facing view direction (−1, 1). Which lot side that is changes with the
rotation.

Ours shows `patterns[3]` for every edge-on diagonal at every rotation,
so at some rotations it is the other side. That is not changed here.
Where the original's diagonal does show (column x = 75 above), neither
side's texels match the original's. That is the texel offset already
recorded as unread, so the screenshots cannot check a side rule. Reopen
with that offset.

**The depth.** For wall kind 0x20 (edge-on), 0x4c7be2 stores −300.
The row loop (0x4c4907) adds it to every row's depth. Each row's depth
otherwise comes from 0x49a5d3, which is `Depth::stand`. Kind 0x10 (the
diagonal face-on, frame 2) and the straight walls get 0.

**The anchor.** With the −300 alone, the anchor has to be halfway
between the wall's two ends on screen, not at its lower end. At the
medium-zoom depth capture (rotation 0, origin [335, −560]) that gives
the original's depth exactly:

- in column x = 75, rows 180, 210, 240 and 270, the one column where the
  original's diagonal shows: 42795, 43012, 43229 and 43446;
- in the columns beside it at both bay ends, where the side walls now
  win as they do in the original.

The anchor itself was not read; it is fitted to these rows. At that
point the height is the mean of the two ends' heights. That is
**unverified**: the Goth bay stands on flat ground.

**Gains (exact colour, whole screen):**

| Camera | Before | After | Notes |
|---|---|---|---|
| Near zoom, rotation 0, [619, −1189] | 61.2% | 61.7% | both bay ends show the shingles; 4 blocks of 50 px went from 0 to 350 matching pixels |
| Medium zoom, rotation 0 | 88.9% | 89.0% | non-green 87.9% → 88.3% |
| Far zoom, rotation 2, [431, −245] | 88.4% | 88.4% | 60 pixels changed: 3 that matched no longer do, and none of the rest match either way |

Still different at near zoom: rows 436 to 520 below the left end. There
the original shows grass and a light sprite where we draw the wall's
foot. That is not this wall's depth or side, and is left to the
near-zoom work.

## 2026-11-15: The Newbie house to tick 562: Generic Sims Call 2, a catalog picture headline; Breakpoint and Show String

**Unverified.** No capture of the Newbie house (`House07.iff`) exists
yet; each piece is ported from the original's code and checked by
synthetic tests. The Goth house's strict output is unchanged line
for line through tick 2881, hash eed688a0… included.

The base neighborhood has two families, the Goths (House05) and the
Newbies (House07); its other houses hold none. In strict mode the
Newbie house stopped at tick 245; it now runs to tick 562.

**Generic Sims Call routine 2** (0x44fb51 to 0x44fbd8). The stack
object must be on the lot (the lot's object table, +0x18 of the
running object's lot +0xdc; native error 0xa otherwise), checked before
the running object is cast to a person (error 0x1c). If the person has
a running action (+0xa84 not 0, the test the route's end uses), the
stack object becomes that action's icon (+0xa54). The branch is true.

**Set Balloon/Headline kind 7** (0x44b811 to 0x44b8d8): a catalog
picture, picked by byte 2 as it is, not with the temp option bit 12
adds (the code keeps both at 0x44b7d6 and adds the temp to one):

- 0: the stack object's;
- 2: the object a local names, the local being operand word 0 shifted
  right by one, signed; not below the frame's local count (frame byte
  6) is native error 0x33, and a negative local fails loudly, as the
  original would read before the locals; so does a local that still
  holds half of an old pointer in the frame storage, which the original
  reads as it is;
- 1: a neighbor's (0x4256f4, the neighborhood record's +8), which is
  **not ported** and fails loudly: whether a record holds its picture
  is presentation we do not model;
- any other byte ends the call.

An ID that names nothing on the lot (not positive, or a null entry of
the lot's table) ends the call too, after the priority is written, as
for an icon with no sprite. The picture is the definition's (+0x11c),
loaded into its cache (+0x8c, 0x45728d) when first asked for, which is
presentation. The record's frame count becomes 1 (0x46dfe9), so option
bit 11 makes the duration 12 ticks a loop. The rest is the other kinds'
path: backdrop, overlay, flag, duration, redraw, wake.

**A head looking at an object that is not a person** (0x59058f to
0x5905d0) was ported here and, at the same time, by #156 for the Goth
house's tick 2837; #156's port is kept. The two disagree on the order
of the coordinates, and **the disassembly reads as the opposite of
#156's**: the caller copies the object's +0xf4, then +0xf8, into the
point it hands 0x454f5c, which stores 3/16 of the first word (`fild
[esi]`) as the target's first coordinate and 3/16 of the second (`fild
[esi+4]`) as its third. +0xf4 is our second coordinate (`animator::place`,
`direction.rs`, `route.rs`), so the target would be `[3/16 y, 0, 3/16
x]`, as for a person's model matrix (`head::placement`), where #156
writes `[3/16 x, 0, 3/16 y]`. The swap changes the Goth house's hash at
tick 2881 (eed688a0… becomes cdd5b72c…; every tick's line is the same,
so only the animator's head state differs so far). It is left as #156
has it, and the strict output is unchanged; **reopen** with track 3,
and with a capture past tick 2837, which would settle it.

**Breakpoint and Show String** (opcodes 15 and 21), which `sims
census` counts 1,489 and 27 times in the install's trees, are ported
whole from the general table (0x45246a); no house has reached either
yet. Breakpoint (0x451e34) reads the word operand word 1 (scope) and
word 0 (index) name, with the reader every primitive uses (0x453141;
a scope above 38, or a negative one, is native error 9), and a word
that is not 0 is native error 0x3e. Show String (0x451ab7) formats a
string from the running tree's file and sends it as signal 0xeb, which
none of the game's eight subscribers takes (each handler's cases were
read: 0x41637f, 0x43c0e4, 0x44302e, 0x46d2c2, 0x47b22a, 0x4a82db,
0x4c3ea5, 0x4f65bf), and its string lookups fail on nothing; so it
changes nothing kept, and its branch is true.

**What stops the Newbie house now.** At tick 563 person 137's head is
measured while its base clip (`a2o-standing-loop`) moves a bone its
foreground clip (`a2o-disgusted2`) leaves alone: the skin's blend of
the two layers (0x5992b3) is not ported.

## 2026-11-15: The pie menu is ported from the original's code

`sims_ui::pie` lays out, hit-tests and draws the pie menu, and groups an
object's actions into submenus, as the original's menu window
(cTSPieMenu, vtable 0x674b54) and the object picker do. It is pure: the
host passes the click, the screen size and the action names (from
`Lot::pie_menu`), keeps a `pie::Pie`, and gets back the chosen action's
index. The 3D head in the middle is the host's to render and pass in.

**Read from the code.** Layout 0x5ea8e1, drawing 0x5eb04c, the label box
0x5eae63, the hit test 0x5eb7c9 and placement 0x5ebab2. The people panel
(0x518665) makes the game's two menus and sets what differs from the
constructor's defaults: radii 60 and 30 (not 40 and 20), a dead zone of
35 pixels, a reach of 150, resource 800 (`PieFace1.bmp`, 201 by 201, all
colour key) in the middle, and the colours (boxes 0,40,140; labels
165,195,214; the highlighted label white, or 0,255,255 while a button is
held, which is also the title's colour). The picker (0x48d56c, 0x48d8a0)
puts a name with a `/` into the submenu named by the text before the
first `/` (exact match), shows at most 12 entries, and opens a submenu
centered on the entry that opened it, with the first menu's entries
hidden. A press in a submenu's dead zone goes back to the first menu.

**Checked.** The window origin and label box agree to the pixel with the
two menus the input track read from the running original ("Sit" and "Go
Here" on the Goth house, `crates/sims-ui/tests/pie.rs`).

**Unverified.** How the drawn menu looks has not been compared with a
screenshot: whether the box fill is opaque (the surface's fill, function
0x40, is taken as opaque), the text's exact position in the box, and how
the head sits in the face. The picker pages past 12 entries on an answer
of -3; what sends -3 is not traced, so only the first 12 are shown. The
menu window's own sounds (`UI_piemenu_appear`, `_highlight`, `_select`)
play only when its flag +0x17c is set, and the people panel never sets
it, so the game's pie menu plays none of them.

**Reopen** when a screenshot of an open pie menu disagrees, or when the
paging trigger is found.

## 2026-11-15: The multi-tile factory; the school bus comes; strict mode to tick 2881

**Unverified.** No capture covers any of it: the Goth captures end at
update 1000, and all of this is from tick 2703 on. Strict mode's output
for ticks 1 to 2702 is unchanged line for line, hashes included:
3b539cc1… at tick 1102 and c5803e84… at tick 2702. (The entry below
gives older hashes for those ticks; `boot.rs` pins the current ones.)
Strict mode now runs through tick 2881, hash eed688a0…, and stops at
tick 2882.

**Time.** Every update is two game seconds: 30 an in-game minute, 1,800
an hour, 43,200 a day. This is measured, not assumed: the load starts at
06:30:44 (globals 0, 5, 6) and tick 2881 ends at 08:06:46, which
`boot.rs` checks. Strict mode covers 96 minutes of the Goths' day. A
full day is about 15 times that.

**The multi-tile factory** (the group branch of 0x440b2d, from
0x440b63) is `Interpreter::create_group`. It is ported from the old
prototype (which compared it with the original's code run alone) and
read again from the original's code:

1. The definition asked for must be its group's catalog master (word
   11 is -1). Anything else makes nothing in the original and returns 0
   (0x440b90), which fails loudly.
2. The plan: the lead tile first (the first definition of the group
   with word 63 set, 0x43acf5, else the one at the group's origin with
   level 0, 0x43ac5c), then every other tile in the file's resource
   order. A group with no lead fails loudly (0x440c9b).
3. Each tile takes the lowest unused ID. It is built as an ordinary
   object and then joined to the end of its lead's list (0x4227b3). Its
   place in the group comes from its definition (0x422872, 0x43183c).
4. The lead takes the master's price in data word 41 (0x440cd9).
5. The group's reset (0x423175 with 1): every tile is reset and its
   thread restarts at Main, the offsets from the lead are worked out
   (0x422a3b), and then, tile by tile, Init runs, the tile gets its first
   turn, and its scheduler flags are refreshed.
6. Each tile still in the group gets the ready flag (0x440d07).

Not ported: the paths for an allocation or registration failure, and
object flag 0x80, which nothing in the simulation reads.

**What the bus reaches next:**

- Create New Object Instance turns a new group to the stack object's
  facing by the group's own turn (vf 0x4c, 0x422937, at most four
  steps). The group comes off, each tile turns by the tile's data word
  23, the offsets are worked out again, and the group goes back if it
  fits. If it does not fit and the place is on the lot, it turns back
  first. A one-tile turn (0x433f75) still fails.
- The place test of a group (vf 0x54, 0x422abb). The origin is the
  place less the tile's offset, and an origin off the inner tiles
  refuses with code 1 (0x422632). Then every tile is tested at its own
  place. The code kept is the first tile's, replaced by a later 0x1e or
  0x1f. A tile that leaves no code fails loudly, and so does a later
  tile's unknown code after a known one. In a collision test, every
  tile of my own group counts as me (vf 0x5c, 0x42314e).
- A level outside 1 to 3 is now its own refusal, code 2 (0x433058).
  Before, a level of 0 was taken as on the lot. The hashes did not
  change.
- Find Location For modes 3 and 4 (0x44b676) search along a line through
  the local object, by its facing (mode 4 turns it by 2). The line steps
  0, 1, -1, 2… along one coordinate (0x434be9). Without flag bit 0 they
  give native error 0x3d. Modes 2 and 5 still fail.
- An entry that runs at once, pushed onto another person, runs on that
  person's stack (0x45d240). 0x45d20b is the person's own method and
  reads nothing of who pushed it.

**After the bus:**

- **Tick 2747: Special Effect** (opcode 35, 0x44aaa6) is a machine on
  the primitive state. State 0 sets global 11 to 1 and yields. State 1
  needs a stack object (native 0x17) and reads the duration. It then
  yields if the effect is shown, and otherwise takes the true branch;
  any later state is true. The effect is shown with flag 8, or when the
  options' check box (options +4, 0x4cfe18) is on and the zoning or
  family gate passes. That check box is written only by the Options
  panel (0x4e62fa) and by the constructor (0x4e5f63, with 0), and the
  frozen image holds 0, so it is taken as off. The listeners' message
  0x10c, the text and the effect itself are presentation. That they
  write nothing the sim reads is **unverified**.
- **Tick 2791: Snap's false branch** (0x454e09 to 0x454e9b). Person 16
  stands up from the sofa (objects 173 and 174) onto a place that
  collides with tile 174. Word 52 becomes 0, or 2 when the placement
  error is 1 (off the lot). Word 54 becomes the ID of the object
  collided with, or 0. Snap clears the error and the object first
  (0x454d64), and only a collision (0x433b42) writes the object, so a
  room or wall refusal leaves 0. Two cases fail loudly:
  - A person blocked by another person with flag bit 1, which asks
    0x45bd5e (not ported).
  - A multi-tile object that does not fit. Its group test keeps the
    first tile's code, but a later tile's collision still writes the
    object.
- **Tick 2836: animator states 1 and 2** (0x58f0eb, 0x58f12e). A clip
  still in the foreground is cleared, the noted posture is forgotten and
  the base layer is updated, unless the state is 1 and person word 78
  is set. Either way the animator stops walking. The route fields
  cleared with it (+0x24, +0x28, +0x290) are not kept.
- **Tick 2837: a head looking at an object that is not a person**
  (0x59058f). The target is the object's position in world units (3/16,
  0x454f5c) at height +0.0. A person looking at itself still fails.

**Where it stops.** At tick 2882 person 16, standing after the sofa,
has a pose without a foreground layer (0x5992b3). That needs the base
layer's playback, which is new animator state, and the skin's pose
from a base layer. It is big, so strict mode stops there.

**The bus itself.** At tick 2703 the carpool (object 8, tree 0x1017
node 4) makes the School Bus (`SchoolBus.iff`, master 0x50AC15BF). Its
lead is object 12 (GUID 0x50AE1B48) at (728, 488), and it has 30 tiles,
3 across and 10 long. In the same tick a second group is made,
`CarJunk.iff` (lead 334, GUID 0x4B748E3B, 18 tiles). Deleting a group
(0x440f37) is still not ported and is not reached by tick 2881.

**The snack order** now runs through step 3400. At step 3401 person 16
(tree 0x1013 node 1) routes for an action that wants to be interrupted
(0x45afd2), which is not ported.

## 2026-11-15: A person is taken off the lot and deleted; Snap turns a slot's offset; Create mode 6 takes flag 0x20

**Unverified.** No capture covers any of it. The Goth captures end at
update 1000; the removal is at tick 1104, the turned Snap at 2515 and
the new Create at 2703. Strict mode's output for ticks 1 to 1102 is
unchanged line for line, hash 9b09ff4f… included, and the K=1000 pins
are unchanged.

**Who leaves.** Person 259 is the traveling salesman
(`npc_travelingsalesman`, GUID 0x44A86A6C, neighborhood record 301).
Object 382 makes him at the pedestrian portal in tick 6. At tick 1104 he
walks to the portal and removes himself (global tree 0x0176: Go To
Relative Position, then Remove Object Instance on himself, node 27).

**Taking a person off the lot** (virtual function 0x50 with 1, 0x433125,
shared by the person's vtable 0x667378 with an object's) is the one-tile
take-off an object gets, plus one call: the person's animator (virtual
function 0x2c, 0x4594de) is refreshed (its function 0x54 with 1,
0x58da12). The refresh returns at once unless animator byte +0x109 is
set (0x58da2c), which is whether the house view's last draw found the
person on screen (0x590b8b). That is presentation and not kept, and the
animator's tick writes it too. It is not always set for a drawn person:
in the captures the salesman has 1 at update 900 and 0 at updates 950
to 1000. We take the "not drawn" branch, which is **unverified**. When
it is set, the head's step (0x590768) advances the head's clock and
counts person word 45 down, which a script can read. So only the cases
where that cannot be read first are taken:

- Remove Object Instance takes a person only when it removes itself and
  yields (not with flag bit 0, not in a synchronous tree). Its own tick
  follows at once and runs the head's step at the same clock, and the
  removal at the end of the update destroys it.
- Removal at once (0x44123f, which the end of the update also runs for
  the queued person, refreshing it a second time) deletes the person
  right after, so only the deletion's last reset could read what the
  refresh wrote. That it does not is **unverified**.

Every other way of taking a person off fails loudly, naming 0x58da12.
Whether the head's step done twice at one clock (refresh, then tick)
counts word 45 down once or twice is not read; a reader before the end
of the update is the only way to tell. What could still differ is also
read by a head looking at the queued person (0x59293d) or by a person
still queued after a partial pass, and both fail loudly.

**Deleting a person** (0x440eeb) runs in this order:

1. It gives up the interface events (0x444bc8).
2. A selected person would be deselected (0x445449), which is not
   ported and fails.
3. Every person lets go of it (0x444306).
4. Its last reset runs (virtual function 0x38, 0x45e791) with the
   deletion mark (object flag 0x40). The mark skips the move back onto
   the lot (0x45e95d). The reset also erases the person's route
   contexts (0x45e8aa, 0x45fad7) and keeps its path. That erase holds
   for every person reset, which before failed on any context.
5. Each object it holds is removed at once (0x441196).
6. Its sounds stop, and it leaves the object list and its type's
   instance list (0x44383c). The person's destructor (0x459511) frees
   the route, the animator, the uses, the contexts, the path and the
   queue. So the World drops its animator, route, tick state and unknown
   data words, and the lot's list of people loses it.

The deletion writes no neighborhood record, relationship, family or
room. Other objects' frames and relationships that name the ID keep it,
as in the original. The animator's reset now stops a route
(0x5892dd → 0x589e2d, result to 0 at 0x5892e4). For a result other than
1 or -1, the destination becomes the unset position (0x589e5d). A
removal before the load has worked out the lot's rooms, a person with
a routing frame count, and a person on several tiles all fail loudly.
A removed person's room keeps its occupied mark, which is the
original's behaviour. Create mode 8's removal of a new person that does
not fit now goes through the same path and takes the false branch.

**Snap with an offset** (0x454b93 to 0x454c54) works in world units:

1. The object's position becomes `[3/16 x, 0, 3/16 y]` (0x454f5c).
2. The slot's offset becomes `[3/16 o1, 0, 3/16 o0]` (0x454ecc).
3. The offset is turned by the matrix of the raw facing word (0x455070,
   0x5c3a20 axis 1; 0x5c4510 with every term in the original's order).
4. Each turned coordinate is added to the position and stored.
5. The sums are rounded back half away from zero (0x454fb9).

The cosine and sine come from libm, with the caveat `Ext::cos` already
gives. Routing slots use the same helpers. The bound on how far out a
position may be is gone, since the round trip is now computed.

**Create New Object Instance mode 6 with flag 0x20** turns the new
object to the stack object's facing with bit 0 cleared (0x44e640). This
is mode 8's code, shared. A turn (0x433f75) still fails. Mode 6 never
places the object (0x44e72c). Mode 6 making a person now fails loudly
at 0x44e60e, where before it would have skipped the person's traits.

**On Goth.** Strict mode runs through tick 2702, hash 8b89f19d….

- At tick 1104 the salesman is removed (hash at 1103:
  1849f00c…).
- At tick 2515 object 21's Snap (tree 0x2018) turns its slot's offset.
- At tick 2703 the carpool (object 8, tree 0x1017 node 4) makes the
  school bus (GUID 0x50AC15BF), a multi-tile object, and the factory
  for multi-tile objects is not ported. That is big, so strict mode
  stops there.

The snack order removes 259 at step 1104 and stops at the same school
bus at step 2703 (tick 91146).

After the Goth social (the pie-menu run in `boot.rs`), the salesman
leaves at tick 597 while his animator plays "A2O-door-knock". The reset
that deletes him would remove that clip, which can fade it out
(0x592878), and that run stops there.

## 2026-11-15: A headline icon keeps its priority and duration; the social ends

**Unverified.** No captured update shows a headline: the Goth captures
stop before the social. The port is read from the original's code.

Set Balloon/Headline (0x44b724) now shows an icon, kind 6 among them.
It touches only the object's first headline record (object +0x148,
0x3c bytes each), so only one per object matters. The priority is 2 for
kind 4 and 1 otherwise. A record still showing with a higher priority
is left alone; otherwise its priority (+0x34) is written first, even
when the call then ends without showing anything. A duration of 0
clears it (0x46e042). Otherwise, from 0x44b7ed:

- The index is byte 2, plus my temp 0 (object +0x3a) with option bit
  12. Kind 7 is a catalog picture (0x44b811) and still fails loudly.
- The sprite is the kind's first sprite plus the index (the table at
  0x6c1f98, 0x416a0c): kind 6 is 600 and up. Kind 0 has its own table
  (0x666758), and kinds above 10 have none. No sprite ends the call.
- The frame count is the sprite's, from the headline sprite cache
  ([0x6f0c08], made at 0x4144c4 from `GameData/Sprites.iff`), divided
  by three (0x46dfb5). If that is 0 the call ends.
- The duration is kept as given, -1 included. With option bit 11 it is
  multiplied by the frame count and by 12 (0x46e050).
- The object is redrawn (0x434101, presentation) and woken (0x44574b).

The sprite cache (0x4c8e0d) looks up an SPR# and then an SPR2 of the
ID as resources (live chunks of the default variant, through the
resource map when there is one), not the file's raw chunks. The count it
hands on (0x4c8deb) is 0 below version 500, halved for versions 501 and
503, and divided by five for 506 (0x4c154e). `sims_vm::icon_frames`
reads every count once at boot into `Program::icon_frames`. An SPR#
header is swapped by the resource layer's callback (0x4c8f63) unless the
resource's conversion flag 0x10 is set (0x41d3a7): big-endian without
it, little-endian with it. We read the flag from the chunk header;
whether a resource map's entry can carry different flags is
**unverified**. All 195 SPR# in Sprites.iff have flags 0 and read
big-endian.

What the simulation reads of the record is kept in `World::headlines`
(appended; snapshot format 24): priority and duration, while the
duration is not 0. Its readers are:

- Sleep's passive test (0x44aa57), which needs no showing headline.
- The tail of an object's tick (0x4521f7 → 0x46e004), which counts a
  positive duration down.
- A person's reset (0x45e8af) and the destructor (0x4323ce), which
  clear it.

The sprite, the animation frame, the backdrop, the overlay and the
flag (+0x14 to +0x28, +0x2c, +0x30, +0x38) are only drawn and are left
out. That the drawn bounds (0x437ce8, 0x45a07b) feed nothing the
simulation reads is **unverified**.

Two small stops after it are ported too:

- **set_action with person word 73 set** (0x45e756): the word is
  cleared and the person's footprint worked out again, 0x433d16 with
  +0xf4 and +0x100. This is the same call Generic Sims Call routine 15
  makes (0x44fe9e), so it reuses that port. A facing the footprint does
  not handle fails before the action changes.
- **Go To Routing Slot, wanting to be interrupted** (0x45b18b, the
  test before the search for places): if the animator may stop
  (virtual function 0x30, 0x589ead), the route fails with result 4.
  The animator may stop in states 1 to 3; in states 0, 9 and 10 the
  foreground layer's +0x68 decides, or it may with none; in any other
  state it may not (`Animator::interruptible`). The same test at a
  door, on the way and while waiting (0x45afd2, 0x45b2c2, 0x45b480)
  still fails loudly.

Strict mode: through tick 1102 every line of `sims boot` matches main
except the final hash, now
3b539cc1f0cfddbfdc8e12a0e864e90528333317095306cf9463c9b7de5a6f1e. The
headlines stay empty in all those ticks. An empty map encodes as one 0
byte, and the encoding without that byte digests to the old
9b09ff4f78d81be329ef6d64a062bd6d1e4f72a530b977a3df83c57220cf5b18. The
tick-33 pin is re-pinned at format 24; its format 23 and 22 digests are
the old pins. The K=1000 tick pins in `resize.rs` hold unchanged.

**On Goth.** At tick 261 tree 0x200e shows "+ " (kind 6, index 16:
sprite 616, 9 frames, so 3) over person 21 for 100 ticks, and at tick
262 over person 16. The social ends at tick 264: word 73 is cleared and
the footprint worked out again. Both icons count down and stop showing
by tick 361. The lot then runs to tick 597, where person 259 (GUID
0x44A8C46C) fails its route with result 4 because it
wants to be interrupted. Tree 0x0176 node 27 then takes it off the lot,
which is not ported. That is where the next port starts.

The old prototype's `headline.rs` agrees on the kept fields and the
countdown. Reopen if a capture shows a headline's duration counting
differently, a drawn bound reaching the simulation, or a resource
map whose SPR# entry has flags other than its chunk's.

## 2026-11-15: A turn of less than a step is made at once; Run Functional Tree's flag names the action's icon; Find Location For mode 1

**Unverified.** No capture covers any of the three. In the Goth captures
(updates 1 to 1000) no person ends an update in state 3 short of its
destination's direction, and no direction moves by less than a step
without a turn clip.

**The small turn.** State 3 compares the turn d with the step (a quarter
over the speed) and then the distance to the destination (0x58dfc0). An
animator that has arrived is 0 away (0x58df1a). Below 0.5 (0x6d07e4) the
animator is put at its destination at once (0x58dfcf): the position
takes the destination's two coordinates, the direction the destination's
(virtual function 0xd0, 0x58488b), and data word 1 the direction
(0x5920e8). Then state 3 runs again (0x58f089), which finds it arrived and
aligned. No clip plays. Global 0x6f6d64 becomes 4, which is not kept.

The step clip by heading (0x58f312, the jump table at 0x58fad6) and the
clip turn with a distance (0x58f4ee) come only after an animator that has
not arrived. From 0.5 up to 3.1 (0x6d07b0) away it takes the step, and
from 3.1 up it turns with a clip. State 3 short of its destination still
fails at 0x58df1e, so neither is reachable and neither is ported.

**Run Functional Tree's flag.** The person's virtual function 0x34
(0x45e59b) takes a flag as its fourth argument. With it set, the object
becomes person +0xa54, the icon of the running action at +0xa48
(0x45e5d4). The use entry keeps the flag (entry +8). When a use ends with
its frame (0x45e64c), each entry with the flag, first to last, writes its
object there again (0x45e65d to 0x45e665), so the last one's stays. An
entry ended by a shallower return writes nothing (0x45e678). Of the
ported callers only Run Functional Tree passes a flag. Run Tree By Name,
Idle For Input (0x45dca2), Gosub Found Action (0x45a920) and the route's
pushes pass 0.

**Find Location For mode 1** (0x44b519 to 0x44b565) takes the stack object
off the lot: its virtual function 0x50 with 1, as the removal queue
calls it, then the true branch. A tile of a group (definition +0x14 set)
is first tested with 0x42344e and may go to 0x42346a. That is not
decoded, so it fails loudly, as Remove Object Instance does.

**On Goth.** In the snack order Mortimer's animator is 5.7e-6 short of its
direction at step 836 (tick 89279) and faces it at once. At step 1035 his
tree 0x1050 runs object 121's function with flag 1, and object 121
becomes his action's icon. At step 1088 his tree 0x1002 takes the snack
(object 263, in his hand) off the lot. Person 259 makes a small turn at
step 1103 (-0.025). At step 1104 (tick 89547) it removes itself (Remove
Object Instance in tree 0x0176), and taking a person off the lot (the
person's virtual function 0x50) is not ported. That is big, so the order stops
there.

Strict mode on the Goth house gives the same output for 1102 ticks. It
now runs through tick 1103 (person 259's small turn) and stops at tick
1104 on the same removal. The K=1000 pins are unchanged.

## 2026-11-15: Scope 24 reads neighbor fields; field 9 is the family friend count, cached in the family

**Partly verified** (the load's count and the threshold, against the
capture of the Goth house's first tick; the reads during the social pick
are **unverified**).

Scope 24, "neighbor in stack object" (0x453389), hands the stack object
and the field to the neighborhood's 0x425b5d. That finds the record by
its place plus one (0x4256f4), as scope 32 and Set To Next do. No
record gives 0 for every field, where scope 32 throws 0x37. The field
switch (0x425d41) has ten cases; above 9, or negative (compared
unsigned), is 0. The value has no word pointer: with write intent the
caller's pointer is cleared (0x425b8e), so a write is native 0x13 after
the field is read.

- **2** is person data word 58 (+0xe8); **3** and **4** are 0.
- **6** walks the families (neighborhood +0x154) for the first whose
  members (+0x12c, 0x4101f2) hold the record's GUID, and gives its house
  (+0x110), whatever the record's word 61 says.
- **9** finds the first family whose ID (+0x10c) is word 61 (0 gives 0)
  and, with or without a house (0x425cfa does not read +0x110; the house
  checks are fields 7 and 8's), returns 0x410260 → 0x427219: the
  family's cached count (+0x120), worked out again first by 0x427bd7
  when the stale byte (+0x124) is set.
- **0**, **1** and **5** read the record's +8 (a pointer to the person's
  type data: 0x423f30 reads +0xb4 from it, 0x4572af +0x60). **7** and
  **8** read the family's flags (+0x138), whose bit 1 the statistics
  pass rewrites (0x4160f9), which is not ported. All five fail loudly,
  naming their address.

The count (0x427bd7) walks the records in slot order. It skips a record
whose GUID is a member's, or whose word 61 is 0. For each member GUID it
takes the first record of that GUID (0x42571f, none for GUID 0). The
record counts once, at the first member it is friends with both ways
(0x4271c5: each has a row for the other with at least one value, and
both first values are at least the friendship threshold), if its kind
(word 65) is at most 1. The threshold is 0x6c4398: the autonomy tuning's
"friendship threshold" (default 25.0), stored truncated (`__ftol`,
0x459120). The capture holds 50 there, and our tuning reads 50. Then the
stale byte is cleared and the count stored.

The family's constructor (0x41003c) sets the stale byte. The FAMI reader
(0x4104ce) stores the saved count but leaves the byte. It is set again
only by 0x4246b9, which a relationship write calls for both records
(0x44485b): it marks the record (+0x24) and the first family whose ID is
the record's word 61. (Family moves, 0x425f66 and 0x42601e, set it too.
They are reached only from Generic Sims Call routines that are not
ported, and from the interface.) The count is not a function of the
current state: a change to anything else it reads (word 61, word 65, a
GUID, membership) leaves the old count until a relationship write marks
the family.

So the World now keeps the families (`World::families`, appended;
snapshot format 23): ID, house, members, count and stale flag, in the
reload's order. The members are filtered to GUIDs some record has, as
the reload's first step already said. Budget, number, flags and the rest
of the record are not kept, since nothing ported reads them. The
relationship write now marks the family, which the earlier port left
out.

Load step 20 is the interface's family panel (0x4cfcdb) showing the
count, so the load works out the selected family's (global 9) count.
In the capture at the first tick the Goth family (5) has count 0 and is
up to date, and the other 25 families have the file's 0 and are out of
date. Our state matches, and `resize.rs` pins it. For an empty lot
nothing is counted (unverified).

**The panel's redraw.** The panel shows the count again whenever the
game updates its windows. The redraw is slot +0x17c of the
`cWinViewControl` vtable at 0x66f220 (stored at 0x66f39c), 0x53a01f,
which the generic window update 0x5dce19 calls at 0x5dd0de. It builds
the label through 0x4cfcdb → 0x410260 → 0x427219, which works the count
out again if it is out of date. The game updates its windows as it
handles its messages: between lot updates, and inside the pump that
waits for a dialog that stops the game (0x436da0 → 0x4f6b95 →
0x5dab4b). So `Interpreter::refresh_family_panel` counts the selected
family again after each of `Lot`'s lot updates and as the registry
shows each such dialog.

This matters for more than the stale flag. A count is cached until a
relationship write marks the family, and a relationship read makes
missing rows (zero values) without marking it. With a threshold of 0, a
write, then a redraw that counts 0, then a read that makes rows, then a
read of field 9 gives the cached 0 in the original, where a family left
out of date would count 1. A test covers the cache across a redraw.

**Partly verified.** In all 608 captured updates of the Goth house,
family 5 is up to date after the update; only update 2 changes
relationships (record 32's row for 34, from [50] to [50, 0, 50], which
marks family 5), and update 2 shows a dialog. The snapshots do not show
which redraw cleared the mark. Whether the window update redraws the
panel on every pass, or only when the panel is marked, is
**unverified**. Redrawing more or less often differs only when a
relationship write and a read of field 9 fall between the same two
redraws while something else the count reads changes. The tick pins
leave the families out, so they are unaffected.

Strict mode: through tick 1102 every line of `sims boot` matches main
except the final hash. That hash covers the new families field; without
it the state equals main's. The tick-33 pin shows the same: its format
22 digest is the old pin. The K=1000 tick pins in `resize.rs` hold
unchanged.

**On Goth.** The social pick reads field 9 at tick 261 for record 34
(word 61 = 5, the Goths). The social's relationship writes to records
32 to 34 had marked family 5 again, so it counts again: 0. The social
then stops in the same tick on object 21, tree 0x200e node 21: Set
Balloon/Headline with an icon of kind 6 (0x44b7ed), which is not
ported.

The old prototype's `family_friends.rs` agreed on the count, the cache
and the marks. It also kept the family flags and a per-family dirty
plan. Reopen if a capture of a relationship write shows the family's
stale byte untouched, or a capture shows a redraw at a point other than
between lot updates and a dialog's wait.

## 2026-11-15: Person word 1 goes to the interface and is cleared

**Unverified.** No capture has a nonzero person word 1.

The person's tick hands a nonzero person word 1 to 0x4a69aa and then
clears the word (0x45ce61 to 0x45ce7e). 0x4a69aa shows the value over
the person, in one of three ways:

- Below -20000 it shows -20000 less the value as text (0x4d356a).
- From -20000 up to -10000 it plays the sound "sting_char_winner" for
  the person (0x57a54c) and shows -10000 less the value (0x4d3516).
- From -10000 up it shows the value as money (0x4d34f7).

Each is shown only while the interface's object ([0x6f2a58]+0x84) is
there. Showing means drawing text into a bitmap over the person
(0x4d322c), which is presentation. So the tick now only clears the word.
The middle range still fails loudly. Its sound could be a
`SoundEvent::Play`, but nothing shows whether the interface object
exists in a run with no view, and the snack order does not reach it.

The play-mode tests that set word 1 to 1 to get a fault outside the
thread now set -15000.

**On Goth.** In the snack order, Mortimer's word 1 holds 5 after step
820's scripts, the amount he just spent. That step now finishes (tick
89264). His carry clip
reaches its end two steps later and stops. At step 836 (tick 89279),
walking with the snack, his animator turns by less than a step
(0x58dfc0), which is not ported. Person 259 stopped at the same place,
at step 1104, while Mortimer was parked. That path decides between a step and a walk by distance
(0x58dfc0, 0x58f312) and picks a step clip by heading (the jump table at
0x58fad6), so it is not small. Strict mode on the Goth house (1102
ticks) and the K=1000 pins are unchanged.

## 2026-11-15: The animator's own carry clip starts in the carry slot; a changed skin sorts its layers by priority

**Unverified.** No capture has a person carrying with the carry clip the
tick chooses. In the Goth captures (updates 1 to 1000) only person 21
has a carry layer, a script's (carry mode 6, from update 417), and the
captures hold no layer pages.

The animator's tick chooses the carry clip at 0x5903a3. A change of
choice clears the carry slot at once (virtual function 0x104,
0x592878). It no longer fails when a layer is there, since the tick's
own carry layer is one. A change to clip 0 starts the clip at once
(0x5903fc to 0x590421):

- It looks up the posture's clip 0x26 (virtual function 0xf4, 0x592625,
  which reads the appearance again when stale, then 0x59db2f).
- It starts that clip in the carry slot (virtual function 0x100,
  0x592726) with flag 0: forward, speed 1, end mode 0 (0x592823), so the
  clip stops at its end. 0x592726 sets end mode 0 for every slot. Only
  Animate Sim's carry path then sets 1 (0x58977c).
- It gives the layer priority 10 (0x59867f), which marks the skin
  changed.
- The carry mode stays 1, not 6, so each tick chooses again. Empty hands
  or a pickup clip (+0x1a4) choose -1, which removes the layer.
- Later updates are those of any carry layer: the skin's update, with
  timed events handled.

The skin's update sorts its layers by priority before it updates any of
them (0x5991fd, called first by 0x5992b3). The sort is a bubble sort,
lowest priority first, and runs only when the skin has changed (skin
+0x94). Layers of the same priority keep their order. Every change of
layer or priority marks the skin changed, so the layers always update by
priority. The 2026-11-14 carry entry kept only the insertion order
(0x599128). So a carry layer (10) now updates after a foreground layer
of raised priority (6), not before it.

`Animator::carry_first` stays the insertion order and is not written by
the sort. It decides only a tie. The original's list keeps the sorted
order instead. The two differ only when two layers had different
priorities at a sort and the same priority later, and nothing ported
does that: foreground layers take priority 1, 5 or 6, carry layers 10.
Keeping the field as it was leaves snapshots and the K=1000 pins
unchanged. Writing the sorted order into it changed the tick 417 digest,
where person 21's script carry layer plays with a foreground, though
nothing else differed. No format bump is needed.

Still failing loudly: a carry clip with no name (0x59db2f) or one no
file holds (0x59dc32). The original leaves the slot empty then and
still notes clip 0 as chosen.

**On Goth.** The snack order's step 820 (tick 89263) now starts
Mortimer's carry clip, `a2o-rarm-carry-loop`, at priority 10. The rest
of his tick stops on person word 1 (5, the money just spent), which the
person's tick hands to the interface (0x4a69aa). Strict mode on the Goth
house (1102 ticks) is unchanged. Reopen when a capture holds a carry
layer the tick chose, or the skin's layer list.

## 2026-11-15: Alter Budget spends into today's budget entry; a thread's Add/Change Action String names the running action

**Unverified.** No capture covers the snack order.

Alter Budget (0x44eade) now spends: when bit 0 of byte 4 is clear and the
amount is covered, it calls 0x46d91c with the category and the amount.
That routine does nothing in budget mode 1 (global 32). In any other mode
it takes the amount from the funds (simulator +0x90), adds it to the
category's word of today's entry (+0x184), both in 32 bits that wrap, and
writes globals 14 and 13 as the funds setter (0x46d8dc) does: the funds
split at 10,000 by signed division, cut to 16 bits. The setter writes only
in mode 0; this writes in mode 2 too. Then it sends signal 0xda with the
funds (0x40760d).

Today's entry is the first of `Budget::days`. The simulator keeps six days of
eight words: today at +0x184, yesterday at +0xe4, and the four before it
at +0x104 to +0x164. The load (0x46d1e4) reads the save's first day into
+0x184 and the next five into +0xe4 on, so the save's order is the one
`days` already had. Per day the save holds a flag word, then eight words
if the flag is not 0, or seven if it is (0x46d270). The game always writes
the flag as 1, since the same code writes and reads (0x464970). The
parser reads no words for a flag of 0, so a today with flag 0 fails
loudly. So do category 8, which the primitive allows (0x44eb10) but which
adds to +0x1a4, the head of a list that the constructor allocates
(0x46ce0c), and a lot with no days read from a save. Each fails before
anything changes. The Goth save's days are today
[20, 0, 0, 0, 0, 0, 15060, 33654] and five days of zeros.

Signal 0xda is presentation. The game has eight subscribers (the
constructors that call 0x407577): the lot (0x41637f), 0x43c0e4, 0x44302e,
the simulator (0x46d2c2), the architecture (0x47b22a), the house viewer
(0x4a82db), the panels (0x4c3ea5) and the interface (0x4f65bf). Only the
interface has a case for 0xda. It marks the control panel's funds with
0x4000 and redraws it (0x4ce41b), which formats the funds as text
(0x4ce81c, 0x4cecf5). The lot's handler first passes the signal to lot
+0x10, but by a direct call to the empty base handler (0x41639e calls
0x407607, which returns 0), so it does nothing for any signal. That was
unknown before (see `tick.rs`).

The day's turn (0x46d417) is still not ported. When it is: it moves the
days with a forward copy that overlaps, so +0x104 to +0x164 all end up
with yesterday's old words before today moves to +0xe4 and is cleared.
A save of version below 0x34 is reset after it is read (0x46d211 calls
0x46d97a: every day cleared, and global 28, the day count, set to 0). The
load does not do that yet. It matters only for the base game's version
0x33 template houses.

Add/Change Action String (0x44cc07) in a thread's run now names the
person's running action instead of failing. Outside a pie menu's listing
it checks the byte 0x6c4d6c, which starts as 1. Only the synchronous run
(0x4749b5) changes it: it clears it for the run and puts it back after
(0x474a37, 0x474ad0). With the byte set and the object a person
(0x635abe), 0x41f1ae stores the string as the action's name (+0x28) and
sets its flag 0x80 (+0x40). As before for 0x41f1ae, only the flag is
kept. In every run, the string is looked up (0x4704fa) and formatted
(0x44bf1a) before 0x44cc9a asks where it goes. A formatting pass can
raise an error; one raises native error 0x1c for a non-person
(0x44c57b). So a missing string, or one with a "$" token, now fails
loudly in every run, autonomy's check trees included. Before, it failed
only in a pie menu's listing (a Sol review finding). A string without
tokens is not formatted: only the name would hold the result. Another
object's thread does only the lookup. Autonomy's check trees then drop
the string. Other runs at once still fail loudly, because the port's runs
at once have not been matched one by one to 0x4749b5's callers (0x422d7d,
0x44bc1b, 0x44f593, 0x452c37, 0x458505, 0x474b0b).

In play mode, after the Have A Snack order (50 steps, then the fridge's
menu 1), Mortimer's tree 0x1007 spends 5 in category 3 at step 820 (world
tick 89263): funds 2542 to 2537, today's category 3 from 0 to 5, global
13 to 2537. In the same tick tree 0x104a names his running action. His
animator then stops on the carry clip: "a person starts carrying: its
carry clip (0x26, 0x5903fc) is not ported". An install test pins this.
Strict mode on the Goth house is unchanged. Reopen when a recording of
the snack order disagrees, or when a script spends in category 8.

## 2026-11-15: Generic Sims Call routine 15 copies temp 0 into person word 72; the footprint is worked out after word 73 is cleared

**Unverified.**

Routine 15 (0x44fe3c; the Generic Sims Call jump table at 0x45143a)
casts the running object to a person (0x635abe; a failed cast throws,
native error 0x1c). It copies temp 0 (object +0x3a, the word routine 28
reads its lot from) into person data word 72 (+0x61c) and clears word 73
(+0x61e). When temp 0 is 0 it then works out the person's footprint
again where it stands (0x433d16 with +0xf4 and +0x100, its width through
0x43519f). That reuses the footprint that placement already ports. Word
73 is cleared first, so the footprint never takes the even-facing
extension from it. When temp 0 is set the footprint is kept, whatever
word 73 held before.

The port works the footprint out before it writes anything, from the
person's words as they will be, so a footprint that fails loudly (a
facing outside 0 to 7) leaves the person as it was. The original cannot
fail there.

The Goth house's social pick runs it once, at tick 48, on person 21 with
temp 0 at 8, so only the copy and the clear are reached. The footprint
branch is checked only by the synthetic tests. No capture covers the
pick. The social now runs to tick 261, where it stops on scope 24 field
9 (a neighbor's family's friend count, through the family friend-count
cache). That needs state the World does not hold yet. Reopen if a
capture of the pick disagrees.

## 2026-11-15: Animate Sim on the base slot is the carry slot's twin; a script's base clip stays until the posture changes

**Unverified.**

Animate Sim with flag bit 0x01 alone (0x58957e; the slot picker 0x47ec4c
returns 1) works on the base slot (animator +0x14) the way flag 0x10
works on the carry slot. It never yields and always takes the true branch
(0x589c4e). Clip 0 clears the slot (virtual function 0x104, 0x592878).
The base slot's layer is always removed at once, even where the other
slots could fade (0x592890). Any other clip is looked up (virtual
function 0x12c, 0x5893b3), the slow flag (+0xa8) set from flag 0x80, and
the clip started in the base slot (virtual function 0x100, 0x592726),
backward with flag 0x02, to repeat at priority -10 (0x59867f). That is
what the posture's own base clip gets. Unlike the carry slot, no mode or
selection is written.

The base slot used to be a flag, since only the posture's clip went
there. It is now `Base`: empty, the posture's clip, or a clip a script
started, with its name, direction and the animator's clock at its
start. Drawing plays it from that start (0x597a88: the layer's clock
starts at the animator's, its phase at 0, or 1 backward), not from the
clock's zero. `Empty` and `Posture` encode as
`false` and `true` did, so existing states and snapshots hash and decode
as before. The animator's base update (0x590430) treats a script's layer
like any other: a change of posture clears it. Otherwise it stays for a
person of kind (word 65) at most 1, and for any other kind the posture's
clip replaces it, unless bit 2 of definition word 102 is set. Animate Sim
does not touch the posture noted (+0x198), so after clip 0 the next base
update starts the posture's clip again. The head's measurement checks a
script's base clip against the foreground as it checks the posture's.

Beyond a script's start, the base layer's clock and its place in the
skin's list are still not kept. A new layer goes in before the first
layer above priority 1 (0x599128) and only then takes -10, so it may
come before or after the foreground layer; the skin's update sorts its
layers by priority first (0x5991fd), so the base updates first anyway. That matters only for timed events, so a script's base
clip with timed events fails loudly (0x5992b3), as the posture's does.
The original goes on past a clip it cannot find (0x452bed reports it);
here that fails, as it does for the other slots.

**On Goth.** The social person 16 picks clears object 16's base slot
(clip 0, source byte 2, which clip 0 never reads) in the next update.
The lot then runs 46 updates and stops at tick 48 on object 21, tree
0x2025 node 13, Generic Sims Call routine 15 (0x44fe3c). Strict mode on
the Goth house is unchanged. Reopen when the harness records a social,
or when a script plays a base clip with timed events.

## 2026-11-15: Snap mode 4 snaps to one of the lot's global routing slots; an action pushed to the front goes first

**Unverified.** Snap (opcode 46) mode 4 (0x454ab6) names its slot by
word 0 in the lot's global routing slots, the table Go To Routing Slot's
source 2 reads (the stack object's lot, +0xdc, at +0x9c: the kind 3
entries of the global file's SLOT 100). From 0x454ac5 on it is mode 3's
code, so the place is still the stack object's and every later step is
the same. The flags are the lot's one copy, kept under object 0 as Go To
Routing Slot keeps them. An index outside the table, a negative one
included, is native error 0x1b. Modes 0 to 2 still fail loudly.

Queueing an action with flag 2 (0x45d322 to 0x45d47f) no longer fails.
The action has already taken its number. With flag 0x40, the person's
running action (+0xa48) loses its flag 0x80 (0x41f167), and the new
action takes that action's name (0x41f1ae). Taking the name sets the new
action's flag 0x80, which is ported; the name itself is not, since the
queue does not keep names. A queue
of the routine's own takes the new action and then the queued ones in
order, and the person's queue is given them back. A full queue passes
only its first seven, and only seven are given back, so the new action
goes in with the first six and the last two are dropped, with no object
told. The action is always queued, and the branch never looks at an
action running at once (0x45d5a5), so a push to the front from inside one
is queued too.

In play mode, after the Have A Snack order (50 steps, then the order),
Mortimer (object 21) snaps to himself with global slot 5 at step 820
(tree 0x0127 node 2). Its flags are 0x1010, so he turns by 4 where he
stands, from facing 6 to 2. In the same tick the fridge's tree 0x1018
(node 10) pushes his action 4170 on the snack (object 263) to the front
of his empty queue. He now stops in that tick at Alter Budget spending 5
in category 3 (0x46d91c, tree 0x1007 node 2). Strict mode on the Goth
house is unchanged. Reopen when a recording of the snack order disagrees,
or when a push to the front meets a full queue.

## 2026-11-15: The advertisement record outlives Find Best Interaction's checks

**Unverified.** The captures do not hold the record's page.

The original keeps one advertisement record. It is the static record at
0x6e5100, and [0x6c437c] always points to it. The frozen memory image
shows the pointer and a count of 16; the advertisements themselves are on
the heap. The check 0x458319 copies an entry's advertisements over the
record (0x4763c5) only when its caller passes somewhere to keep them.
Find Best Interaction does (0x45dc8b); the pie menu (0x45871f) and the
at-once queue (0x45d259) pass 0 and copy nothing.

The order inside 0x458319 is:

1. An entry ruled out by the target's flags, the visitor rules or the
   person's age and kind returns before the copy (0x458497).
2. The copy.
3. The check tree, if the entry has one. With no tree the check passes
   (0x458524).
4. When the target is in another room than the person, the room motive's
   delta becomes its minimum negated, in the record (0x45853f). This
   happens whatever the check returned. Both rooms are read after the
   check tree (0x45852f, 0x458536), so a check that moves either object
   counts. Before, the port compared the rooms from before the scan.

Scopes 12, 28 and 29 (0x453a09, 0x453a42, 0x453a7a) read [0x6c437c] in
any tree, so a check tree the pie menu runs reads and writes whatever the
last Find Best Interaction check left. The record is now a World field,
`advertisement_record`, and the scopes use it everywhere. Their index is
bounded by 16 (native error 0x1a), as before. This replaces the
check-tree-only advertisements and the loud failure outside a check tree.

0x4763c5 copies as many advertisements as both the entry and the record
have, and the record's count stays 16. So an entry with fewer than 16
leaves the rest as the last copy left them. That case used to fail
loudly and is now ported.

Before the first copy, the scopes and a copy of fewer than 16 still
fail loudly: the record then
holds what the game's start left on the heap, which nothing here reads.
On Goth, object 18's pie menu at tick 1 stops there, before autonomy
first runs (tick 232). Reopen when a capture holds the record's page, or
when a recording picks from a menu that reads it before the first copy.

Snapshot format 22 adds the record at the end of the World. Each pin of
the Goth ticks 417 to 1000 now reads through format 21 and holds as it
was.

## 2026-11-15: Run Tree By Name modes 0 and 1 on my thread; an action queued during an at-once run takes its place

**Unverified.**

**Run Tree By Name.** Mode 0 (0x44bb75) runs the named tree of the stack
object's type on the running object's own thread. I stay me, the stack
object stays its stack object, and my temps are the arguments. Mode 1
runs it on the stack object's thread, so mode 1 with myself as the stack
object is the same run. The tree runs at once, to its end, and its result
is the branch. 0x4749b5 treats the running thread like any other: it
pushes a stop frame and the tree's frame above my frames, runs until the
stop frame, and then puts back the thread word it cleared. The port runs
the tree above my frames in the same way.

**Queueing during an at-once run.** While a person's action runs at
once, an action queued on that person is copied over the running action,
including its number and flags (0x45d5a5, 0x41edbf). It is also queued,
unless the queue is full. The at-once run then returns the queued
action's number and takes none of its own. This replaces the loud
refusal recorded in the at-once entry below.

With flag 0x40, the original also gives the queued action the running
action's name (0x41f167, 0x41f1ae), which sets its flag 0x80. The flag is
set here, but queued actions' names are not kept.

In modes 0 and 1 a check tree's advertisements (the global record at
0x6c437c) stay the same record through the named run.

**On Goth.** The first pick on person 16 now queues the social through
the object its tree made, and returns that action's number. The next
step stops on Animate Sim on the base slot (0x58957e). Reopen when the
harness records a social pick.

## 2026-11-15: Grab puts the stack object in my first slot; a person's slots include its own five

Grab (opcode 4, 0x451dda) ignores its operands. It looks the stack
object up in the registry (0x40743b) and takes the false branch when it
names none. Then it asks the stack object's place test (virtual function
0x54) for my slot 0 as scripts count it, at my position and level, and
on a refusal takes the false branch with nothing changed. Otherwise it
puts the object there with the lot told (virtual function 0x58) and
takes the true branch. Only three classes of object exist, found from
the three callers of the base constructor 0x43189d. An ordinary object
(vtable 0x666d34) and a person (0x667378) share 0x433311 and 0x433340,
so `fits_in` and `put` serve both. A tile of a group (0x666c20) has
0x422abb, which refuses any container at once (0x422ac8, code 0x10), so
Grab takes the false branch for it; its test on the ground still fails
loudly. A put that fails partway puts the lot back as it was before the
Grab.

A person's container slots, which the place test reads, are its
definition's (0x431aaf) followed by five records its post-constructor
adds (0x45962b). Each is made by 0x46de93, which gives an offset of 0
and a height of 0, and only the height or the flags are then changed.
None goes through the SLOT converter that turns a height of 0 into 1 or
5. In order: height 6, the slot a person carries in; height 0 with flag
2 and the name "HEAD" (0x6c4528); then three at height 1. The place
test's step for a person as the container (0x432e57) is ported with
them. A container whose definition type is 2 takes nothing unless the
simulator's mode (global 19) is 0 (error 0xd). With the slots ported,
Create New Object Instance mode 4 no longer refuses a person as the
container.

The snack order's fridge tree 0x100d (node 9) runs Grab for Mortimer on
the snack (object 263) the fridge made. In play mode, after the Have A
Snack order at tick 51, he takes it into his carry slot (saved slot 1)
at tick 748. The snack's data word 4 is 189, which has the bit for height 6. He
now stops at tick 839, on Snap mode 4 (0x4549c7, tree 0x0127 node 2).
Strict mode on the Goth house is unchanged. **Unverified**: no capture
covers a Grab, and nothing has compared a person's slot records with the
original's memory. Reopen when a recording of the snack order
disagrees.

## 2026-11-15: An action whose entry runs at once is checked and run there and then

**Unverified.** Queueing an action whose interaction has flag 4 (0x45d20b,
the branch at 0x45d240) no longer fails. When the person queues it on
themselves, its check runs (0x458319). If the check passes and no action
is already running at once (the pointer at 0x6e5294 is clear), the action
tree runs straight away. It runs on the target object, with the target as
stack object, and takes seven parameters: the four arguments, the two
halves of the priority, and the tree. While it runs, the person's priority
(person data word 33) is the action's. Afterwards the priority is put back.
An action with no number then takes the next one. The person's temps are
restored if the check saved them (0x4582dd). Nothing is queued, and a
failed check, or a nested run, returns no number.

Two cases still fail loudly, because the original does something the port
does not model:

- an object other than the person queues such an action on the person;
- an action is queued on the person whose action runs at once. The
  original copies it over the running action (0x45d5a5).

A fault inside the tree becomes Unsupported. Every pick on Goth's person
16 now gets past this point and stops at Run Tree By Name mode 0 on
another object (0x44ba26). Reopen when the harness shows a pick that runs
at once.

## 2026-11-15: A route to a slot with a sitting weight looks for seats, and a diagonal slot turns through a matrix

**Unverified.** The search by distance (0x483598) now handles a slot with
a sitting weight. It tries each place as indices -1 to 8: first the place
itself, when standing weighs at least as much as sitting; then a seat one
tile away in each of the eight directions (0x4838e7, 0x422632, 0x43244d);
and last the place itself again, when standing weighs less. A seat must
offer Sit (function 26), face back toward the place, and have that
direction's bit in definition word 66. If someone else holds the seat,
result 9 is noted and the holder goes in context word +0x8c. A candidate
beside a seat ends the route with the seat's Sit tree, not a walk
(0x45c430, 0x45b96b), and the route then waits for that tree's return
(step 4, 0x45b3fa).

A slot place turned by a direction other than 0, 2, 4 or 6 goes through
the original's rotation matrix in world units (0x483d7c, 0x455070,
0x5c4510, 0x454fb9). The cosine and sine come from libm, so they may
differ from the x87 `fsin`/`fcos` in the last bit of a float. Seats for a
person of kind above 1 still fail loudly (0x46de84), as does a slot with
flag 0x10000.

This moves a player's snack order on the Goth house from tick 154 to
tick 615. There it stops at Create New Object Instance mode 4. Its seat
search picks a place to stand, so the Sit route is covered only by
synthetic tests. Reopen when a capture of a route that sits disagrees,
or when a rounding at x.5 after the matrix lands on the other side.

## 2026-11-15: Create New Object Instance mode 4 makes the object in the stack object's first slot; flag 0x1 skips a tile with its type

Mode 4 of opcode 42 (0x44e3a2) takes the stack object as the container,
and its position, level and room (data word 29). The new object goes
into the container's first container slot (slot 0 as scripts count it).
Before it is made, the common tail (0x44e4c5) runs three tests, any of
which takes the false branch:

- with flag 0x1 (0x44e4e1), an object of the same type stands on the
  container's tile, or is held there however deep (0x43f331, 0x43f379);
- the position is off the lot (0x422632);
- the container's room is not -5, and the lot's room map gives its tile
  another room (0x44e582).

Then come the slot's place test (`fits_in`), the removal of an object
the slot refuses, and the put. Flag 0x1 is the same tail code for every
mode, so it is ported for modes 6 and 8 too. Mode 6's tile is off the
arrays, where it finds nothing. Mode 8 keeps the room filter at -5, so
its room test never runs. Making a person, a type that belongs to a
group, a person as the container, and a container that the new object's
Init removes still fail loudly, as do the other modes and flags.

The fridge's snack tree (0x1019 node 1) runs mode 4 with flag 0x1 on
the fridge. In play mode, after a player's Have A Snack order at tick
51, Mortimer now puts the snack in the fridge at tick 634. He stops at
tick 748, on Grab (primitive 4, tree 0x100d node 9). Strict mode on the
Goth house is unchanged. **Unverified**: no capture covers a mode 4
creation. Reopen when a recording of the snack order disagrees.

## 2026-11-15: A check tree names pie menu entries with Add/Change Action String

While the pie menu's listing checks an action, Add/Change Action String
adds an entry to the menu (0x458789): a copy of the action being checked,
with flags 4 and 8 set and 0x10 clear, the string as its name, and the
string's four arguments, the first being the frame's stack object when
the formatter left it 0 (0x44cca3). The entry goes into the menu at once,
whether or not the check passes, and the action itself is then left out
(0x458724 compares the list's length). The menu's entries are now
`MenuEntry` values, an action and an optional name; picking one queues
its action. The queue does not keep the name, which the original keeps in
the action (+0x28) for the interface.

The string's formatter (0x44bf1a) is not ported. Each of its passes looks
for a token starting with "$" and does nothing without one, so a string
with no "$" comes out unchanged and so do the arguments; a string with
"$" fails loudly. A string the running code's file lacks also fails
loudly, since what the original does with the null string 0x4704fa
returns for it is not read. On the Goth house the fridge now offers "Have
Quick Breakfast", "Have Breakfast" and "Serve Breakfast", and each
staircase "Go Upstairs"; 85 objects offer entries, and 9 fail in check
trees on Manage Inventory operation 4 and scopes 24 and 12. Reopen when a
string with a token is reached.

## 2026-11-15: A pie menu that offers nothing lists the other objects on the tile

When the listing on an object offers nothing, the original lists each
other object on that object's tile in turn (0x48d423, walking the tile
with 0x43f331 and 0x43f379, the links read again after each listing,
nothing outside the lot's size) and adds what they offer. Each listing
checks again whether the person is a pet. It leaves out an
object of type 8 and one whose definition word 91 lacks bit 0. Each of
those listings saves and restores the person's temps itself, and the
first listing's temps go back last. A menu that still offers nothing
is an empty menu, not a failure. The tile is the one the object's
position is on; for an object held in a slot, whether the original's
object +0xfc names the same tile is **unverified**. On the Goth house
283 objects now give an empty menu instead of failing, 75 offer
actions, and 19 fail in check trees on Add/Change Action String,
Manage Inventory operation 4, and scopes 24 and 12.

## 2026-11-15: An empty interaction table keeps its version and loads

A TTAB chunk with no entries was read as version 0, which the
interpreter refuses as a table older than 7. The original's reader
(0x475cc0) reads the version whatever the count, and the loader
(0x475a24) succeeds with no entries. So an empty table now has its own
version, and loads as a table with no entries. A chunk that ends before
the version word, which the original reads past its end, is rejected;
the install has none. In the Complete Collection install there are 270 such chunks:
225 of version 9, 44 of version 8, and one of version 6
(PoolLadder.iff), which the refusal of tables older than 7 still
catches. On the Goth house this moves 32 objects' pie menus past the
refusal; most now reach the tile fallback for an object that offers
nothing (0x48d423).

## 2026-11-15: The lot lists pie menus and takes picks on the real lot

`Lot::pie_menu(person, object)` and `Lot::pick(person, action)` run the
interpreter's listing and pick on the lot itself, not on a copy, because
the original's listing runs check trees that change the lot. A recording
of play has to note each listing, at the moment the original makes it:
when the cursor first reaches an object (0x48c834), not on the click. A
listing or pick that fails on something not ported changes nothing and
does not halt the lot, as for `Lot::queue_interaction`. The listing
returns the sound requests its check trees made with the actions.

On the Goth house the selected Sim can list and queue entries on several
objects. Every entry on another person still fails on a queued action
that runs at once (0x45d240). Reopen when a recording shows a listing at
another moment, such as on a click.

## 2026-11-15: People let go of an object that resets or is deleted

Before an object resets, and when it is deleted, each person on the lot's
list of people, in its order, lets go of it (0x444306 calling the
person's reset 0x45e791 with the object; `crate::forget`). Objects that
are not people are not told, and a person object not on the list is not
told either. A person that does not use the object filters it out: the
queued actions naming it are dropped, each one's target told it was
cancelled (function 4, so an action on the object tells the object
itself) and the rest kept in order; the last action naming it becomes
the empty action, or loses the icon; the running action naming it ends.
When the object is a person, route contexts asking it to move forget it.
A person nested in a broadcast it set off is skipped, as the original's
object flag 0x800 skips it; that flag is carried by the interpreter for
the broadcast's length, not kept in the world. A person's own reset sets
the same flag for its length, so its callbacks' broadcasts skip it too,
and a reset of a person already flagged, which the original skips, fails
loudly. A broadcast while the factory makes a person fails loudly, as
any walk of the list of people does then. A cancel's fault keeps the
tree and node it happened at, and a failure partway leaves the queue no
longer than its ring of 8.

A person using the object, or whose running action shows its icon, is
reset fully by the original: its uses unwound and each used object reset,
then its own reset. That is **not ported** and fails loudly.

What the filter leaves naming the object, the original leaves as a
pointer to freed memory: the last action after the running action ends,
a route context aimed at an object that is not a person, frames whose
stack object it was. Here they keep its ID, which a later object may be
given. A deletion with a route aimed at the object still fails loudly
before anything changes. Reopen when a capture shows a deletion with
people on the lot; none of this is **verified**. Ported from the
original's code; the old prototype's `person_reset.rs` reads it the same
way. Play mode reached it at tick 1023 after a player's snack order.

## 2026-11-15: The game draws people into the lot with sims-people's depth test

`sims-game` no longer draws the people into an image of its own laid
over the lot. Each frame it draws the lot with `sims_render::draw`, then
the people with `sims_people::People::draw_on_lot` into the same canvas,
which tests every pixel against the lot's depth, then the panel. Walls,
furniture and the floor above now hide the people behind them, at every
zoom and rotation; placement, terrain height and the light are
`sims-people`'s. People on a level above the camera's are not drawn,
and the camera's level stays the player's choice (Page Up and Page
Down), so on the second floor the people downstairs are hidden with
their rooms. How the original gives a person's mesh its depth is
**unverified** (see "People's meshes go into the lot renderer's canvas").

A click finds the person shown at that pixel in the frame on screen (or
the one shown nearest within 2 pixels): while it draws the people, the
game notes whose pixel the canvas took last at each point. A person
hidden behind a bookcase is shown nowhere, so it no longer takes a click
meant for the bookcase, and one off the lot or upstairs of the view is
not found. `sims-people` does not say which person drew a pixel, so the
game repeats `draw_on_lot`'s loop from its public parts (`posed`,
`skin`, `draw_part`, `View`); a test checks that the frames are the
same, colour and depth. Reopen when `sims-people` reports the person
per pixel.

## 2026-11-15: Go To Routing Slot names its slot by parameter or from the lot's global slots

Operand word 1 of Go To Routing Slot picks the slot table (0x45ac22): 1
is the object's table by the literal index, as before; 0 is the
object's table by the index held in that parameter; 2 is the lot's
global routing slots by the literal index. The global table is the kind
3 entries of the global file's SLOT 100, which the original copies into
the lot (+0x9c) as it loads (0x440635, 0x4430e9). A global slot is still
placed relative to the stack object. Any other source, or an index past
the table, is native error 0x25.

The route's flag 0x4000 is written into the lot's single copy of a
global slot, so one object's route changes it for every object. It is
kept in `routing.slot_flags` under object ID 0, which no object has,
rather than in a new world field, so the world's encoding and hash stay
as they are. The original's bound check on a parameter number (0x45ac34)
never fails, and a number past the frame's parameters reads whatever
follows; that fails loudly here. Signal 0x10b clears the lot's table and
parses it again (0x443090 to 0x4430e9), restoring the file's flags; it is
not ported, and a port must drop the `(0, _)` entries.

A player's order for a snack reached source 2 at tick 154, in tree
0x2156; the Goth house's first 1000 ticks do not use it and are
unchanged. **Unverified** against a capture.

## 2026-11-15: The pie menu is listed as the original lists it, and listing changes the lot

`Interpreter::pie_menu` lists what the original's pie menu offers a
person on an object (0x48d392, 0x45859c): every entry of the lead tile's
table in order, each made into an action by its menu number (find-first,
so a repeated number repeats the first entry's action), checked by
`Machine::check_chosen`, and kept when flag 8 is set and 0x10 clear.
Nothing is greyed out; a failed entry is just absent. `Interpreter::pick`
queues a listed action unchanged (0x48dd67).

Listing is not a pure query: the check trees run synchronously on the
person and their effects stay, apart from the temps, which are put back
once at the end; person word 50 is left as the last check set it. The
original lists the menu when the cursor reaches an object (0x48c834), so
a caller wanting parity lists it at the same moments, and a recording of
play must note those moments as inputs. The hover's highlight bits
(object +0x114) are not modeled.

Ported from code alone; **unverified** until goal step 4's recordings.
Taken as clear: the interface flag 0x6f2a54, which hides every entry.
Fails loudly: a pet's menu (it reads interface state 0x6e4f68), a check
tree that adds its own entry (opcode 50 in the menu build, 0x458789),
the entry to join another person's joinable interaction (0x45866d), and
an object that offers nothing, where the original lists the objects on
its tile (0x48d423).

## 2026-11-15: Idle For Input starts a queued action; the pie menu's check

Idle For Input, asked to and interrupted with something queued, now
starts the front action (0x45dc14 to 0x45dd1f) instead of failing. The
action is taken off the queue and checked as the pie menu checks it
(`Machine::check_chosen`: 0x458319 on a context made with byte 0 and no
advertisements). That is autonomy's check without its data word 25
rule, with the check tree's arguments 0, 0, 0, 0, and the result left in
the action's flags 4, 8 and 0x10. A passing action's tree is pushed as
Gosub Found Action pushes one, with the action's own arguments, and it
becomes the running action. The frame's primitive state becomes 1, so
when the tree returns the instruction runs again and starts only an
action pushed to the front. A refused action is cancelled on its target
(function 4) and the thread yields. The person's temps, saved before
the check tree, are put back last.

Ported from the original's code alone; no capture reaches it yet, so it
is **unverified**. Two assumptions: a queued action is taken not to join
another (the save does not keep its word +0, which the pie menu's
actions have as 0), and the interface flag 0x6e5134, which would pass
an action whatever its check tree returned, is taken to be clear, as
autonomy already takes it. A tree that cannot be pushed, where the
original cancels the action, fails loudly. Reopen with a recording of a
player's order (goal step 4).

A check tree the file lacks now writes the synchronous run's mark above
the top frame before it fails (0x4749d9), in this check and in
autonomy's, as `Interpreter::callback` already did; its words can
become a later frame's locals. The Goth house's first 1000 ticks are
unchanged by it.

## 2026-11-15: The game's pie menu is a list of the table's entries until the simulation offers them

Superseded by "The game lists the pie menu on the lot when the pointer
reaches an object" above, except where the simulation cannot list a
menu.

Clicking an object (or a person) in `sims-game` lists the entries of
the interaction table `Lot::queue_interaction` queues from: that of the
tile the simulation sends the order to (a multi-tile object's tile whose
definition's word 8 is positive sends it to the tile whose word 8 is
that number negated, as `interaction_target`, 0x435286, does), read with
`Program::interactions`. Each menu number is listed once, in table
order, named by the TTAs string table with the same ID as that TTAB, in
the file that holds it. The game repeats the simulation's choice of
tile and of TTAB ID from the program's public data, because the
simulation keeps both private; it names the entries only when the table
it finds is the very one `Program::interactions` returns.

The original offers only the entries whose check passes for the selected
Sim; that check is not run, so the list can hold entries the original
would not offer, and choosing one queues it anyway. Entries with flag
0x80 are hidden: every Debug and Force entry of the Goths' house has it
and nothing a player is offered there does (**unverified** as the
original's debug flag). The menu is a plain list at the pointer, not the
original's pie: in columns when taller than the view, and as many
columns as fit, the last row then saying how many entries are not shown.
A dialog closes it. Reopen when the simulation can list the entries it
would offer, or `sims-ui` draws the pie.

## 2026-11-15: The game plays the lot's sound requests as it wakes

`sims-game` takes each step's `Update::sounds` and hands them, in
order, to `sims-audio`'s player in the same wake of the window that ran
the step (it wakes every few milliseconds). What is heard comes later by
the output the player has already rendered, up to about 0.2 s. In strict
mode, a step whose frame fails after its tick still hands its sounds
over. `Play`
is `Play` and `Stop` is `StopObject`. The simulation's 16-bit object ID
is widened without its sign (**unverified**: what the original passes
for a negative ID). Until the host takes them, the game keeps at most
4096 requests, dropping the oldest, so a host that never plays sound
does not grow without end. `begin_route` and `end_route`, which Go To
Routing Slot requests, find no event in the install's sound files; the
player records a fault and plays nothing, which may be what the
original does too (**unverified**).

## 2026-11-15: The game can run the lot in play mode; strict stays the default

`sims-game` steps the lot with `Lot::step` by default, which stops at
the first error, so what it plays is exactly what the captures check.
`--play` (`Options::play`) steps with `Lot::step_play` instead: an
object whose tick fails is reset (as the original resets an object after
a SimAntics error) and the lot goes on, so a human can play past the
first unported primitive. A lot played that way is not one to compare
with the original.

In play mode each new error caught is printed and written across the top
for 200 ticks (10 s at normal speed) with a running count. The same
error again at a later tick (or a frame skipped the same way again) is
only counted, so one that recurs every frame does not hold the line
open. The first 256 different errors are listed, later ones only
counted; after 4096 different errors every new one is only counted, as
it can no longer be told from its repeats. A failure play mode does not
catch still stops the lot. An error caught in a step that then fails is
not logged: `Lot::step_play` returns the failure without the step's
update. Within a step the log keeps the order things happened in: the
frame before the first tick, the objects' errors, the frame after the
tick (a single skipped frame on the first step is taken to be the one
before it, which the update does not say).

## 2026-11-15: People's meshes go into the lot renderer's canvas

`People::draw_on_lot` draws a lot's people into the canvas
`sims_render::draw` has filled, as the render track found the original
does: its depth buffer, dumped with Sims on screen, holds only the lot.
Each mesh vertex takes the lot renderer's `Depth::point` for its fine
view position, elevation and level, interpolated across the triangle,
and each pixel goes in by `Canvas::put`'s sprite rule (no farther than
what is there, ties to the person; fainter than 65 left out; soft pixels
blend; depth written so a nearer part of the same body covers a farther
one). Depths are cut toward zero, as the original's conversion does and
as `Depth::anchor` does. The screen position is the renderer's
`Camera::project_fine_f32`, one column right.

Checked in the booted Goths' lot, each person drawn alone (ticks 120 to
480, four rotations, medium zoom, the camera on the ground level, walls
down): their feet show in 44 of the 53 views where they are on screen.
In the three views looked at where a person or their feet are hidden,
a hedge with the porch railing, a bookcase or the bed's posts stands in
front; the others were not looked at. Feet show because a floor lies 65 units farther than the
point under it, and grass, held at its tile's top corner, 80 or more
(200 to 650 at five grass pixels probed). With the camera on level 2
the upper floor hides everyone below it, as it hides the rooms. How the
original gives a person's mesh its depth is still **unverified**; reopen
when a person behind furniture is compared with the original.

## 2026-11-15: Snap into a container slot, the base clips of postures 1 and 2, and K = 1000

Snap into an object's container slot is ported, with the place test for
a slot (0x432d62 to 0x432fb8), from the original's code. Only one
container placement is reached so far: person 21 sitting in object 174
in tick 992. A container slot off the object's centre would move the
person's animator offset (0x454ceb), which is not kept, so it fails
loudly. The steps it does not reach (mode 2's centre rule, the
other slots at the same height, the containers around the object) are
ported from the code alone and **unverified**. The base clips of
postures 1 and 2 are looked up and fail loudly when missing or when
they have timed events. Posture 0's clip is still not looked up, since
every capture shows its layer after a start. A clip that is never looked
up cannot fail, so reopen this if a person's posture-0 clip could be
missing. K = 1000 on the Goth house. That is step 1 of the goal; next is
a full game day.

## 2026-11-15: A person's ground height and model matrix are read from sims-sim

`sims_sim::head::ground_height` and `sims_sim::head::model` give the
terrain's height under a person and their model matrix (0x592132) from a
`World` and an animator position. They only read the world and add no
state, so the world's hash is unchanged. They follow `sims_vm`'s head
model line for line: the same rounding (a half added, truncated in
single precision, NaN to 0 as `__ftol` gives it), the same tile cut to a
signed byte, and the same order of offsets, which the head captures pin.
They also refuse a tile beyond the 64-tile arrays when a lot's size
says more. sims-people now uses them instead of its own copy. `sims_vm`
keeps its private copy for now; it could call these instead. An upper
level still fails (`Unplaced::Level`): the view-table offset is not
ported.

## 2026-11-15: The lot notes its sound requests; sound changes nothing it keeps

Each update now returns the requests the original makes of its sound
manager (`Update::sounds`, `crates/sims-vm/src/sounds.rs`). There are
two kinds: `Play { object, name }` (0x57a54c) and `Stop { object }`
(0x57a6fb). The original never reads anything back from the sound
manager that the simulation keeps, so the events sit outside `World` and
the hash. The Goth house hashes the same at tick 870 with and without
them.

Where the requests come from, read from the code:

- **Play Sound Event (0x452822).** It names an FWAV chunk, looked up by
  ID in the running code's file and then the global file (frame +0xc's
  +8, then +4). Change Suit reads the same record for those two files
  (0x45da81, 0x45da8e), which settles the old prototype's unverified
  "owner, then Global".
- **Go To Routing Slot.** It plays "begin_route" as a route starts
  (0x45b989) and "end_route" after every stop (0x45b3a0).
- **Stops.** Stop All Sounds (0x45256e) stops sounds, and so do an
  object's reset unless it is being deleted (0x4320b4) and the
  destructor (0x4323c9). A person's reset makes no direct call.

In 870 play ticks the Goth house plays 62 sounds and stops 50:
begin_route 21, end_route 18, door_open 4, and so on.

The sounds a clip's events play (footsteps, "sound" keys, 0x59175e) are
not noted yet. They pick their sound from the person's state and the
floor in code that is not ported; that is the next sound work. The
hour for the ambience is global 0. The camera view is the interface's.

## 2026-11-15: A player's order queues the pie menu's action, without its check

`Lot::queue_interaction(person, object, menu)` (`crates/sims-vm/src/
orders.rs`) queues the action the original's pie menu makes for that
entry. The object becomes its interaction target, and the action is
built by 0x41f00e with priority 50 (0x458633), then given flags 4 and 8
as a check that passes leaves them. It is queued unchanged (0x48dd67 →
0x45d20b), as a pick does. The check itself (0x458319 in the menu's mode:
rules on the person and object, and the entry's check tree) belongs to
listing the menu's entries, which comes next; until then an order can
name an entry the menu would not offer. A failed order changes nothing,
since the queue can fail partway (it numbers the action before it finds
it cannot place it). Push Interaction (primitive 13, 0x44cd2b) is ported
from the original's code; reading one word past a frame's parameters or
locals, which its bound allows, fails loudly.

## 2026-11-15: People are drawn over the finished lot, against its depth

The original keeps its people out of the lot's depth buffer. Two dumps
of the running game (far zoom with a Sim standing on the grass, medium
zoom with two Sims on the porch bench) show the Sims in the colour
buffer and only the lot in the depth buffer: plain grass under the
standing Sim, the wall and the porch floor where the seated ones are.
The bench they sit on is missing from the depth buffer too. So the
original draws the lot first, then draws what moves, and what is in
use, over it, tested against the lot's depth.

`sims-render` takes the same order. After `draw`, a caller draws people
a pixel at a time with `Canvas::put` (the sprite rule: fainter than the
cutoff or farther than what is there is left out, a tie wins, the depth
is written so a nearer part of the same mesh covers a farther one) at
the depth `Depth::point` gives a point: the ground rows at the point's
row, the level's offset, and the height curve for its elevation. It is
an object anchor's depth without the rounding, in the same units as
everything else. `Camera::project_fine_f32` is the projection without
rounding, so the people's code does not keep its own copy.

The interleaved draw planned with the people track (people sorted in
with the lot's objects) is left out: the original does not interleave,
and the depth test against the finished lot already hides a person
behind a wall, a counter or a cut wall's stub. The original draws a bench
in use with the people rather than with the lot; here it stays with the
lot and people test against it, which matters only where a person is
drawn in front of part of the bench and behind another part.

How the original gives a person's mesh its depth is **unverified**:
the dumps show where people are not, not their own depth. Reopen it
with a frame in which a person is partly hidden, comparing the hidden
pixels with `Depth::point`.

## 2026-11-15: Play mode resets an object after its error; strict mode is unchanged

For a playable lot, `Lot::step_play` (and `sims boot --play`) catches a
fault in an object's tick and goes on. `Lot::step`, `sims boot` and every
comparison stay in strict mode, which this does not touch: without an
error, a play-mode update is the strict one, and so is the lot it leaves.

What the original does on a SimAntics error is ported (`crates/sims-vm/
src/play.rs`): the thread's catch (0x474ec1) stores the code and sets
flag 4 through the object's virtual function 4 (0x439a4e), and the rest
of the tick runs. The scheduler then reads the error word, calls 0x439a6f
and stops the pass partial with the cursor on the object. With the debug
byte 0x6c2983 clear, 0x439a6f tells the other objects (0x444306) and
resets the object (its virtual function 0x38 with 0); the restart at Main
(0x47480c) clears the error word, so the next pass ticks the object as
usual.

These are **approximations**, play mode only:

- An unported primitive or service is handled as a SimAntics error.
- A fault in the rest of a tick (a person's animator, motives or room) is
  handled the same way, but the rest of that tick does not run. When it
  follows a fault caught in the thread, both are reported and the object
  is dealt with once.
- Where the reset fails (much of a person's is not ported), the thread
  restarts at Main and the animator resets where it can. Where that fails
  too, the object is parked: its scheduler flag 1 is cleared.
- An object that faults again on its first tick after a reset is parked,
  since the original would stop every pass at it and the clock with it.
  An object deleted in a play-mode update after its reset no longer
  counts, so a later object given its ID is not parked for its first
  fault. A deletion made outside a play-mode update is not tracked.
- A frame of the house view that fails is skipped; every failed frame of
  a step is reported.

Faults elsewhere in the update (dialogs, removals, the clock) still halt
the lot. On the Goth house with the K = 1000 work on top, 20,000 play
ticks run through; 47 errors are caught, and four people end up parked by
animator states that are not ported yet. Reopen any approximation when
the code under it is ported; they should then stop being reached.

## 2026-11-15: People join the lot renderer's view through a depth hook

sims-people draws a live lot's people in sims-render's camera rather
than its own (`sims_people::People`, `View`). It depends on sims-render,
and not the other way round, so the render track can take people later
through a trait of its own. Placement: a person's fine position (x, y)
sits at world (−3y/16, 0, −3x/16), and a unit of height is 32√6/3 ≈ 26.13
near-zoom pixels, from the original's house camera (0x49b0de: tilted
30°, 3√2/(16 << zoom) units to a pixel). Checked: a person's root
projects to the pixel the renderer's `project_fine` gives, at every zoom,
rotation and level, for elevations that are whole pixels of the zoom;
one step of altitude lifts a person exactly as it lifts the renderer's
ground (16 near-zoom pixels). The mesh is projected without rounding, so
at other elevations a point can fall a pixel above where `project_fine`,
which rounds the elevation on its own, would put it. In the booted Goths'
lot each person, drawn alone, is drawn where the renderer projects their
position, at all four rotations. People are drawn one
column right, as the renderer draws objects; **unverified** for people.

A person stands on the terrain under their position, found as the
original finds it (0x478dfe, the same lookup as `sims_vm`'s head model,
which is private, so it is repeated here). On an upper level the original
adds an offset from the view's table, which is not ported: such a person
stands at the floor. A person whose object is off the lot (taken off, at
tile −1, −1) is not drawn, wherever their animator was left;
**unverified** against the original.

Depth is the caller's. `People::draw` takes a function from a point's
fine view position, elevation and level to a depth, the signature agreed
with the render track for its coming `Depth::point`, and interpolates it
across each triangle; people are drawn far to near by their root's depth.
How the original gives a person's mesh its depth is **unverified**; reopen
when it is read. Until the renderer takes people, an app draws them over
the lot, and they overlap walls and furniture wrongly.

## 2026-11-15: The game's sound inputs are guesses until measured

`sims-game` feeds `sims-audio`'s player two inputs the simulation does
not own. The sound scripts' speed register (global register 100,
`SimSpeed`) is 1 at normal speed, as `sims-audio`'s `play` example
uses, 2 at fast and 3 at ultra; the ambience's view, which the original
works out from the camera (0x570c3e, not read), is held at 50, the
value that plays the backyard ambience in that example. Both are
**unverified**. The ambience's hour is the clock's (global 0). The
mixer runs on its own thread, a fifth of a second ahead of the device;
when it falls behind, the device plays silence rather than the game
waiting. Reopen when the original's view value or speed register is
read.

## 2026-11-15: The game draws people over the lot with a matching 3D camera

Superseded by "The game draws people into the lot with sims-people's
depth test" above.

`sims-game` draws the people with `sims-people`'s orthographic camera,
built from the lot camera so that a ground point lands on the pixel the
lot's 2:1 projection gives it (within the lot camera's rounding, at every
zoom and rotation; pinned by a test). Heights take the scale a
horizontal length gets; a person upstairs is raised one storey's
screen height, and every person by the terrain under them (the tile's
corners blended by where in the tile they stand). The people are drawn into their own image and laid over
the lot whole, so walls and furniture in front of a person do not hide
it yet; occlusion belongs to the render and people tracks. That the
original draws its people at this scale is **unverified**.

## 2026-11-15: The game paces the lot itself, and runs it strictly

A tick runs every 50 ms at normal speed, the animator clock's step.
Fast and ultra run three and ten times as often (**unverified**: the
original's are not measured). At most 20 ticks run for one wake of the
window; time beyond that is dropped, so a stall never makes the lot
race. Pause stops stepping the lot; the panel's pause button shows it.
The speed and pause stay outside the simulation until it takes player
orders. The lot runs in strict mode with the captured frame schedule
(`Frames::default()`), so it plays exactly the states the captures
check; the first failure stops it and is shown on screen. Clicks pick
objects by their tile until `sims-render` can say which object drew a
pixel. Reopen when the play-mode step and player orders land.

## 2026-11-14: Routing slots count among the kind 3 entries

Go To Routing Slot and Snap now name a routing slot by its place among
the SLOT chunk's kind 3 entries, as the object's routing slot table
holds them (0x46e2d1), not by its index in the whole chunk. Every route
compared so far went to an object whose routing entries come first, so
no earlier tick changes; object 174 in tick 992 is the first whose SLOT
starts with a container entry. The per-object slot flags and records
written over the type's (`Routing::slot_flags`, `Routing::slots`) are
keyed the same way, as the table's index; the marker's first slot is
the table's first. Snap's own checks of the place (0x4550d8 to
0x4551e9) are ported from the code and not yet reached by a capture:
**unverified** until tick 992's snap into object 174 runs them.

## 2026-11-14: Job data from the CARR chunks, a frame after tick 901, and K = 991

Scope 33 reads a field of a career level from the careers `work.iff`
holds, as the original's list is built from its CARR chunks (0x403e3f)
and read by 0x40362f; the field order is ported from that code, and
field 22 (whether the level's second string is not empty) takes the
string's first virtual function as its length, as the old prototype did:
**unverified**, reopen if a script reads field 22. An install with a
`Careers.txt` still fails loudly. The fifth capture's room motives show
that it drew a frame after tick 901, two ticks off the schedule the
earlier captures share, so the comparison draws one there. The schedule
is the original's main loop, not the simulation, so the default
(`0,1,2,3,33+30`) is unchanged; reopen if a capture can record its draws,
which would replace the inferred schedule. K = 991; tick 992 snaps to a
slot whose placement checks the border, the room and the walls.

## 2026-11-14: The hour turns, a new time of day grows the lawn, and K = 900

The hour's turn is ported from the lot update (0x46d3cc to 0x46d51d).
When the time of day changes, the game tells the interface (0x4c4648)
and sends signals 0xf8 and 0xdd. The interface call picks a panel's look.
On 0xdd the lot's subscriber marks every room and sets a light ceiling
only the drawing reads.

Signal 0xf8 is not presentation, as a first reading had it (the review
caught this). The architecture's subscriber (0x47b22a) takes it and,
when global 36 is not 0, grows the lawn (0x47b3c8): each tile inside the
lot's border whose grass (Arry 6) is below its target (Arry 7), and that
has a diagonal neighbour already at its target, grows by 20, up to the
target. The sweep changes the array as it goes, row by row, and adds in
32 bits as the original does. It then sends 0x86 and 0x83: the
interface's panels (0x4c3ea5) set a refresh flag on 0x86, and the house
viewer redraws on 0x83. This is ported (`World::grow_grass`) from the
code; the original's direction table was read from a memory image. If
the lot lacks the two arrays the update fails before the time of day or
the rooms change.

Only the architecture takes 0xf8. Besides the lot, the interface's
panels take 0xdd (0x4c4189: the current mode's time-of-day look, then
0x83). That the panels' and the house viewer's handling of these signals
changes nothing the simulation keeps is **unverified** (the review
corrected an earlier, wrong census).

Ticks 878 to 900 cross 7:00 but do not change the time of day: the save
already holds day (global 4 is 0 at 6:30, in the capture and here), so
neither the lawn nor the rooms change, and they equal the fifth capture.
The first change comes at 18:00. The tick captures do not hold the lot's
arrays, so the lawn's growth is **unverified** against the original;
the full day's recording must capture the grass array at 18:00. The
day's turn still fails loudly. K = 900; tick 901 reads job data
(scope 33).

## 2026-11-14: No headline state until a headline can show, and K = 877

Set Balloon/Headline is ported only as far as clearing (0x44b724). An
object has a headline when its SLOT has an entry of kind 1 or it is a
person, as its construction builds the list; nothing else is kept. The
priority a call writes is read only while a headline shows, and every
call that shows one writes it first; showing one fails loudly. So while
nothing shows a headline, a clear changes nothing that is kept, and so
do the other readers of the list (Sleep's test, the countdown after a
thread runs, a reset). Keeping the list now would be state that nothing
could ever change. Reopen when a script shows a headline: then the first
headline's duration, priority and tick count are kept per object, and
those readers are ported with it. K = 877; tick 878 turns the hour.

## 2026-11-14: The head relaxes as the original's runs do, not as its code reads

With the follow off, the head's turn is multiplied by 0.8 each tick
(0x590886). The code image shows `fmul qword [0x672030]`, a double 0.8,
and nothing else writes the turn (only the step, its settle to 0, the
constructor and the snapshot). Yet in the fifth capture, every relax
step where the two products differ follows the product with 0.8 as a
float (0x3f4ccccd), rounded once: five steps of person 21's, ticks 814
to 833. The other 17 steps give the same result either way, and no step
in ticks 1 to 871 follows the double. Rounding up or a 53- or 24-bit
precision of the double product does not give the captured values.

The captures are how the original is observed, so `Head::relax`
multiplies by the float. Why the run differs from the code is
**unknown**; the captures run the game under an emulator, and its x87
may round its own way. Reopen if a run on real hardware follows the
double, or if a relax step in a later capture follows neither. The
persistent one-unit head target differences (see "Ticks 334 to 366" in
the roadmap) may have a cause of the same kind.

Also K = 871: a carry layer that moves no bone the head reads is passed
over, and Set Motive Change is ported from 0x45a16f.

## 2026-11-14: A room upstairs, and K = 752

The planner's room rectangles (0x4818af) read the level from the room's
ID and take that level's room map and walls; nothing else in them
depends on the level but the ground's covering and water, which count
only on level 1. So routing upstairs is the ground's code with the
level's arrays, not a port of its own. A room ID naming level 3 or 4
fails as invalid state, as the lot holds two. K = 752: ticks 691 to 752
equal the fifth capture; tick 753 needs a pose with a carry layer.

## 2026-11-14: The render uses the original's depth values

The lot is now depth-tested in the original's own depth space: 16-bit,
cleared to 65535, smaller is nearer. Every surface computes the number
the original would store. Checked pixel by pixel against depth buffers
read from the running game at three cameras (memory dumps, never
written):

| Camera | Depth exact | Screen exact (before → after) |
|---|---|---|
| Medium zoom, rotation 0 | 99.1% | 88.0% → 88.9% |
| Far zoom, rotation 0 | 99.7% | 89.9% → 90.6% |
| Medium zoom, rotation 3 (night) | 97.2% | (lighting differs) |

House box at the screenshot cameras, exact in 565 space, before → after:
medium 90.4% → 91.9%, far 90.2% → 92.4%, far rotation 3 90.0% → 92.5%,
cutaway 87.1% → 89.0%, rotation 1 85.5% → 86.8%, rotation 2 86.1% →
87.6%, near zoom 90.4% → 93.8%, 80.5% → 81.8% and 56.2% → 56.9%.

How each surface gets its depth (all exact against the dumps unless
noted):

- **Tables.** Five per zoom, generated from formulas and checked entry
  by entry against the game's own (digests in `depth.rs` tests): a row
  table R (the ground's depth by screen row from the origin), an
  upright table L (height above an anchor), a column table C (across a
  wall), a terrain height curve H, and the 255 sprite depth bytes T.
- **Grass and floors**: R at the tile's row plus H for its height, plus
  80 for grass (the caller's own offset); floors follow the ground row
  by row, plus 65.
- **Walls**: an anchor at the wall's lower end on screen, then L by row
  and C by column (0x4c4907); a wall pixel wins at up to one more than
  what is there. Diagonal walls use no column table.
- **Objects**: the object's anchor (0x4c9ceb) plus the sprite's depth
  byte. The original reads L at the sprite context's own row, so a
  sprite's height above its anchor adds nothing (0x49a5d3).
- **Faint sprite pixels**: a sprite pixel with alpha under 8 of 31 is
  not drawn at all, colour or depth. This is fitted, not read: it is
  the threshold that matches best (alpha under 6 or 9 matches less).

The door's left jamb now matches: the column the wall covered was a
faint door pixel in front of the wall, which the original never draws.
At medium zoom the door boxes rose from 86.4% to 88.5% exact.

**Unverified:** upper-level floors and anchors (the dumps show level 1
only), and the wall depth the original takes off for its mask 32. What
is left at rotation 3 is mostly the fountain, a bench and stairs on the
upper level, and sprite edges.

## 2026-11-14: Rotations 1 and 2 and near zoom, compared with the original

New screenshots of the Goth house in the original: far zoom at rotations
1 and 2, and three near-zoom views at rotation 0 (the front wall and
door, the porch, the bay window), all with walls up. The camera origins
were found by search; each places the house to the pixel. House box,
exact matches in 565 space:

| Camera | Exact |
|---|---|
| Far zoom, rotation 1 | 85.5% |
| Far zoom, rotation 2 | 86.1% |
| Near zoom, front wall and door | 90.4% |
| Near zoom, porch | 80.5% |
| Near zoom, bay window | 56.2% |

The geometry lines up at every one: walls, openings, trim, floors,
objects and grass. Most of what is left is not drawing:

- The shots were taken at 6:49 in the morning, later than the earlier
  ones. In the original the stair room (the one with the bay window)
  is drawn at about light-table level 20 (see `Light`), judged from its
  pixels; ours uses level 16, the half-light floor. Its walls, floor
  and stairs make up nearly all the misses at rotations 1 and 2 and in
  the bay view. The room's light is the simulation's, so the render
  leaves it.
- The Sims in the shots, which this crate does not draw.

One drawing difference remains in these new views (the door jamb's
depth, recorded below, is separate): a diagonal wall seen edge-on (frame 3,
the bay's left end) shows other texels than ours over its width, 4
pixels at far zoom and 16 at near. Showing the wall's other side did
not match either, so it is not only the side. The original picks the
side from the run's direction (0x53fd47), and for an edge-on line looks
up its own side's wallpaper by a fixed view direction (0x53f8ce); the
texel offset was not read. Reopen with that offset.

## 2026-11-14: The original's strip on a marked edge-on diagonal is read, not ported

The last unported piece of the wall caps is the strip the original lays
on a marked cut wall that runs edge-on (a "rising" diagonal, frame 3).
From its code (0x53eb6d to 0x53ebf5, 0x53e87b, 0x4ac14b):

- The strip is always pattern 252 in shape 9, whichever end is marked.
- Before drawing, the strip's rectangle is moved up by 24, 47 or 94
  pixels at the far, medium and near zoom (`(92 >> (3 - zoom)) + 1`,
  or `+ 2` at near). The call is to OffsetRect; other call sites pass
  (rect, -left, -top).
- The rectangle is the wall's: 4 pixels wide at far zoom, from 42 above
  its anchor to 16 below, and an edge-on wall is anchored 1 pixel left
  and 17 pixels lower (at far zoom) than the other kinds.

Put together, the strip would end 8 pixels above the stub it belongs
to (the marked wall's shape 7 or 8 is only the stub for this frame), so
either part of the placement is still unread or the case does not come
up. None of our screenshots has one, and the Goth house has five
diagonal walls, none marked in the compared cutaway at any rotation.
So the original's rule is not ported: if such a wall does come up, we
draw it like any marked cut wall (pattern 252 in shape 9 or 253 in
shape 10, by which end is marked, not moved). Reopen with a screenshot of a cut edge-on diagonal meeting a standing
wall.

Caps on diagonal corners (the 2-pixel slices) are drawn as the corner
records give them; they too are unverified for the same reason.

## 2026-11-14: The removal queue, no way in the planner, and K = 690

Remove Object Instance now queues as the original's removal queue does
(0x442d70), taking the object off through `take_off`. Only while the load
has not worked the rooms out (`room_at` answers that there are none) does
it take the narrower `leave_lot`, which the load was compared through:
the general way looks up the room of the off-lot place, which the load
cannot answer yet. The narrower way now also works the object's
footprint out again at the off-lot place, as `take_off`'s move does
(the factory's may differ: object 44, removed during the load, had
one); anything else it cannot match fails loudly. A fault in a callback the removal runs keeps
the object, tree and node it happened at (`Fault::nested`), as do Snap's
placement faults. Reopen if an object is removed during the load in a way
`leave_lot` refuses.

The first capture to 1000 updates stopped at 609 with a harness error the
spatial-order reader fix (#47) already addressed; a fifth capture with
that reader recorded the full state after updates 600 to 1000, and its
streams equal the fourth's to 608, so ticks 600 to 690 are compared with
it. K = 690.

## 2026-11-14: Light levels use the original's colour tables

Lit pixels were often one step off from the original: window and door
frames, porch floors, walls in shade. The cause was the formula, not the
light. Read from the original's code:

- A sprite's light is a float from 0 to 1. If it is within 1e-6 of 1,
  the sprite is drawn as is (0x4c7c59). Otherwise the blitter turns it
  into a byte, `L = trunc(light * 255.0 + 0.5)` (0x4b76a8), and picks a
  table by `l = L >> 3` (0x438e6d).
- The tables (built at 0x49eea8) map each 565 channel `v` to:
  red `trunc(l * v / 31 + 0.5)`, green `trunc(2l * v / 63 + 0.5)`, blue
  `trunc((max(l, 1) - 1) * v / 31 + 1.5)`. The divisors are the f32
  constants 1/31 and 1/63.
- So blue is not scaled like red: at half light (`l = 16`) a blue of 0
  becomes 1 and every even blue gains one step, and even at `l = 31`
  black's blue is 1. That is the one step we were missing.

`Light` in `crates/sims-render/src/light.rs` now holds the table index
(or full light) and applies these formulas. House box, exact matches:

| Camera | Before | After |
|---|---|---|
| Medium zoom, rotation 0 | 89.3% | 90.4% |
| Far zoom, rotation 0 | 89.5% | 90.2% |
| Far zoom, rotation 3 | 88.8% | 90.0% |
| Far zoom, cutaway | 85.8% | 87.1% |

At medium zoom, lit pixels now match 93.6% (from 89%); most of the rest
are placement and depth.

**Unverified:** the two shrubs by the bay window (and 16 other objects
in the same room, mostly fences and flowers) are drawn at half light
here and at full light in the original. The simulation puts them in room
4, which it marks indoors with light 32. Either the room's outdoors flag
or the tile light the original keeps for unfloored tiles differs; that
is the simulation's data, so the render does not override it. Reopen
when the simulation's room lights are checked against the original.

## 2026-11-14: The door's left jamb is a depth contest, not a mask

At medium zoom our wall covers about a pixel column of the front door's
left jamb. Two findings, from the original's code:

- **The opening is masked the same way.** For door and window styles
  (22 and 23) the original finds the object on the wall's tile
  (0x43595b) and draws the wall in that object's opening shape: its
  wall shape plus 3 times the cut level plus zoom minus 1 (0x53d525).
  We do the same for standing walls; a cut wall at a door or window is
  drawn as a plain stub instead (the cutaway hides the door with it).
  The opening shapes carry no depth.
- **The jamb loses on depth.** In our depth space the wall is 4 bytes in
  front of the jamb's part at that column. The original's wall depth
  comes from per-zoom tables (a row curve plus a column table, about 6
  bytes per row and 4 per column at far zoom), compared `z <= prev + 1`
  in the wall blitter (0x4c4907), while sprites compare
  `candidate <= previous`. Ours has the same slopes but not the same
  rounding.

Tried and dropped, each within a few hundred pixels either way: moving
walls 4, 8 or 12 bytes back; pairing wall rows and columns one step
apart; a smooth per-column wall depth. A real fix is a port of the
original's depth tables at every zoom, which need reading from a running
game. Reopen with those tables.

## 2026-11-14: Read-only accessors for the presentation crates

The render track asked for two ways to read the simulation without touching it:

- `ReadOnlySnapshots` opens an existing snapshot file with redb's read-only
  open: it never creates the file, never repairs it and never commits, so a
  viewer can read a snapshot another program keeps (several readers share
  it; a writer holding it locked refuses the reader). `Snapshots::open`
  still creates the file, for the CLI that writes it. Both share one load,
  with the same format check.
- `World::selected` and `World::selected_whereabouts`: the selected Sim's
  ID, position, tile, level and room. The room is the object's room word
  (data word 29), which the placement writes whenever the Sim is put
  somewhere, rather than a new lookup in the room map, so the presentation
  shows what the simulation holds. `sims_vm::selection::selected` now
  calls `World::selected`.

Neither changes the state or its encoding. Reopen if a presentation crate
needs something the state does not hold.

## 2026-11-14: Wall caps follow the original's corner records

Walls in the original are thick, and their sprites stop at the corners,
so wherever a wall end shows, something must close it off. The original
does this in two places, and both are now drawn. Read from its code;
`crates/sims-render/src/caps.rs` follows it step by step.

**Corner records.** The original groups walls into straight runs. A run
ends wherever walls meet at an angle or a wall stops; those corners have
a record (0x53d2fc's tables). When the cutaway changes it works out, for
each record, which walls stand, which are cut, and which need a cap:

- A cut wall with no standing wall at the corner moves to the cut set.
  A cut wall that meets a standing wall is *marked* instead (bit 2 or 4
  of its flags, one per end). A cut wall next to a standing one along
  the same run is marked at that end too (0x54059a).
- Fences, and cut walls in a door-like style, take no trim or cap there.
  Marked walls take no trim either.
- The walls left over pick a cap from a table (0x53cd4e). A lone wall
  end, and an L-shaped corner whose outside faces the viewer, take one;
  a straight pass-through, or a corner opening toward the viewer, does
  not.

**The cap at a corner** (0x53c6b7) is a slice of a wall face, 3 pixels
wide at the far zoom (2 at some diagonal corners), doubling at each
zoom. It lies across the wall's end, beside the corner, in the plain
wall shape (style 255's for a wall of that style) and the pattern of the
wall it closes, lit like the wall at the corner that lies along it, if
any, else like the wall it closes. It stands on the corner's own ground,
as the original projects the corner's position.

**A marked cut wall** (0x53ea55) is drawn in wall shape 7 or 8 (by which
end is marked) instead of its cut shape. Those shapes stand full height
for a few pixels at the marked end. Over that, the original lays wall
pattern 252 or 253 in shape 9 or 10: a strip of wood trim down the
stub's edge. The strip is drawn at full light. With the wall's light,
the cutaway box scored 85.3% instead of 85.8%, and the trim along the
top is at full light too.

**Diagonal cut corners** (0x53c478). Where cut trim runs along a view
diagonal with bit 0x10 and a wall also leaves along +x (bit 8) or +y
(bit 4), the original lays the cut set's frame again over the right or
left part of the corner. This is drawn but changes no pixel in the
screenshots we have.

The trim at corner records now comes from the same records: standing
walls' trim, less fences, cut door-like walls and marked walls; the cut
walls' trim, less the same. At corners a run passes straight through,
the trim keeps the earlier rule (2026-11-14), since the run's own
drawing (0x53df0b) was not read in full.

**Result** (exact matches, house box and whole screen):

| Camera | Before | After |
|---|---|---|
| Medium zoom, rotation 0, walls up | 88.9% / 85.8% | 89.3% / 86.0% |
| Far zoom, rotation 0, walls up | 89.0% / 89.8% | 89.5% / 89.9% |
| Far zoom, rotation 3, walls up | 88.7% / 86.2% | 88.8% / 86.3% |
| Far zoom, rotation 0, cutaway | 83.8% / 88.4% | 85.8% / 88.8% |

The caps' column was checked one pixel either way at all four cameras;
the scores were the same or worse.

**Unverified:**

- The extra pieces at diagonal cut corners; no screenshot shows them.
- Caps on diagonal walls, and a marked diagonal wall's strip, which
  the original nudges at some zooms (0x53ebc0, not ported).
- Near zoom, and rotations 1 and 2.
- The original keeps one record for a run that is one segment long,
  so a mark made at one end is also seen at the other; our per-segment
  flags behave the same way, but no screenshot shows such a wall.
- Which room's light the cap takes when the corner has walls on more
  than one line; the original searches the corner's walls for one on
  the cap's line (0x53c97f), and so do we.

## 2026-11-14: Held objects stand on their holder's slot

The lamps were missing because we drew them on the floor. The Goth
house has three, all table lamps: one on a kitchen counter and two on
end tables. Each is held by its table: data word 2 names the holder and
word 3 which of its slots. Their own position is the table's, and their
sprites carry no height, so they ended up inside the table and were
hidden by it. The sink in the other counter is held the same way.

The original draws a holder's contents while it draws the holder
(0x437fdd). It adds the slot's offset, turned by the holder's facing,
to the position, and adds the slot's height. We now do the same:

- the slots are the holder's SLOT entries of kind 0 (OBJD word 20
  names the chunk);
- the height comes from the entry's category, as the old prototype read
  it and checked against the original (`containment_height`): counter
  4.05, table and end table 4, low table 2.5, category 1 none, others
  the entry's own z;
- the height adds to the sprite's own height, which raises it on screen
  and moves its depth;
- the offset adds to the held object's own position and level, which
  the game keeps at its holder's (the original starts from the
  holder's; the two agree in this house);
- a hidden holder hides what it holds;
- a loop of holders gives no offset, and offsets and heights are
  clamped to a lot's width, so a broken save cannot overflow anything;
- objects a Sim holds stay where they were.

**Result.** At far zoom, rotation 3, the end-table lamp now matches the
original pixel for pixel in place. Exact matches in a box around it
rose from 62.2% to 78.1% (96.4% within one light step). The house box
rose from 88.5% to 88.7%. The other compared cameras do not show a
lamp: the lamps stand behind walls there.

**Unverified:**
- The kitchen counter lamp is not in view at any compared camera.
- Every slot here has no x or y offset, so the direction we turn slot
  offsets (the same as draw-group offsets) is not checked.
- The original draws held objects right after their holder; we sort
  them by depth with everything else, which puts them after it here.

## 2026-11-14: Objects sit one pixel right of the walls

At every compared camera, objects were drawn one pixel left of where
the original draws them, measured against the walls and floors around
them. Moving every object one pixel right raised the exact matches at
all three cameras. Moving them one pixel up or down, or two pixels
right, did not help. Moving the walls one pixel either way instead
lowered the house box from 89% to 57%, so the walls were already in
place. The shift helps plain and mirrored sprites alike.

| Camera | House box before | after | Whole screen before | after |
| --- | --- | --- | --- | --- |
| Medium zoom, rotation 0 | 74.1% | 88.9% | 76.4% | 85.8% |
| Far zoom, rotation 0, walls up | 75.7% | 89.0% | 84.4% | 89.8% |
| Far zoom, rotation 3, walls up | 74.0% | 88.5% | 80.8% | 86.2% |
| Far zoom, rotation 0, cutaway | 57.6% | 68.0% | 80.8% | 85.7% |

(The cutaway row uses the settled screenshot with no rooms in focus,
so it is lower than the figures in earlier entries.) A search around
each fitted camera origin still picks the same origin.

The renderer's camera origin is fitted to the tile art, so the
constant is `OBJECT_COLUMN = 1` in the object placement. Where the
original's pixel comes from is **unverified**: its tile rectangle
(0x4ac25d) runs from x - half width - 1, which suggests it draws tile
art one pixel left of the projected point and objects at it. The
floor code around 0x4b17b2 shows no such -1, though, so we have not
confirmed it.

**What still differs at the door at medium zoom:** the door's position
matches pixel for pixel on its right edge and handle. One column of
its left jamb (x 466) still shows the wall in our render and the door
in the original's. Our wall there is 13 depth bytes in front of the
door frame, and its doorway mask opens one column later. Shifting the
doorway masks one pixel either way lowers the overall match, so we keep
them where they are; that does not prove they are exact at this column.
The original's wall may have a deeper edge around the opening that a
flat wall depth does not reproduce. Both are **unverified**. Window and door
frames also differ by one light step in places. The original's
screenshot also has a cardboard box by the front door that a tick-1
world does not have.

## 2026-11-14: The sun does not turn with the camera

The grass at far zoom, rotation 3, matched the original only 12.7%.
The reason was the sun. We tilted it by the camera's rotation (1, 2, 3
and 2 tenths of π from the zenith at rotations 0 to 3), as the setter
at 0x4a4551 does. But only a command handler calls that setter (message
0xdd at 0x4c41bf), and turning the camera does not send that message.
The sun keeps the angle the grass's constructor gives it: π/10 (the
float 0x3ea0d973 at 0x49c187), at every rotation. It still stands
toward the view's +x, since the slopes it lights are read in view
order.

Everything else in the shading was checked against the original's code
and already matched:
- view tiles turn to lot tiles through a 64 by 64 table built around 63
  (0x407704), as `unrotate_tile` does;
- corner heights are reordered per rotation (0x6c4ebc);
- the vertex loop and the blade map count from the lot's least view
  corner, which turning the camera sets (0x4791a6) and the shading
  (0x4a4783, 0x4a48eb) and the blades (0x49ce80) use.

**Result** at far zoom, rotation 3, walls up (the private screenshot
from the camera entry below). Exact matches:

| Area | Before | After |
|---|---|---|
| Open grass (x 560 to 760, y 440 to 540) | 12.7% | 99.4% |
| House box (x 230 to 480, y 200 to 400) | 55.2% | 74.0% |
| Whole screen | 22.6% | 80.8% |

Fitting the angle alone gives the same answer: 0.09π and 0.11π reach
about 63% to 65%, 0.125π 28%, and 0.1π 99.4%. Rotation 0 is unchanged:
84.4% of the whole screen with walls up. Medium zoom is unchanged too:
76.4%.

**Unverified:** rotations 1 and 2 have not been shot, but the same
constructor value applies to them. Sending message 0xdd (probably a
debug command) would tilt the sun; it is not modelled.

## 2026-11-14: Snap to a routing slot, and K = 608

Tick 584 stopped at Snap (opcode 46). Person 18 snaps to routing slot
0 of object 207. The slot has no offset and flag 0x800. The capture
shows the person afterwards where 207 stands, on level 2, in room 1025.

**Decided.** Port only mode 3 (a routing slot of the stack object).
Within it, port only the case the house reaches: no snap target, flag
0x800, operand flag 0 clear, and a snapping object that fits. That
object is a person in the house; an object that is not one snaps the
same way, without the animator. Everything else
fails loudly with its address. Each of the other branches needs code
we do not have:

- the placement's own checks without flag 0x800 (`0x4550d8`);
- the container slot for a snap target (`0x454b19`);
- the animator offset that operand flag 0 keeps (+0x8c, read when the
  animator places itself);
- the error words the failure path writes (`0x454e09`), including a
  person-to-person test (`0x45bd5e`).

**The float round trip, kept for a zero offset only.** The original
turns the slot offset by the object's facing through a sine and a
cosine (`0x455070`, `0x5c39a0`). With a zero offset the matrix adds
nothing, and the position goes through ×3/16 and back ×16/3 with
round-half-away. A coordinate times 3/16 is exact in a float. The
error of 16/3 as a float moves the result by less than 0.001 for any
coordinate up to 0x8000, so the round trip keeps the position. A
non-zero offset (the third coordinate is never read), or a position
further out than that, fails loudly.

**Order of writes.** The order follows the original:

1. the place test, then "set object" with the lot told (0x433340);
2. the facing;
3. the animator unset (0x589361);
4. data word 52 (+0xb2) cleared;
5. the animator's offset zeroed and the animator placed again (its
   virtual functions 0x2c and 0x60).

The animator's +0xd9 byte and the offset fields stay unmodeled, as
before.

**The upper level's floor test** (`0x4330b7`) is ported as one more
refusal. It has two codes, 0x12 and 0x29, which differ only by the
level the player is looking at; no caller tells them apart.

**Result.** The boot equals the capture through tick 608. Tick 609
stops in removing object 260.

**Reopen** when a later tick reaches a branch that fails loudly.

## 2026-11-14: Get Direction To, and K = 583

Tick 467 of the Goth house stopped at Get Direction To (opcode 12).

- **Ported from the original's code** (`0x452003`). The destination
  is resolved for writing first, as Random Number does. Then come the
  object to look from (my object data word 11, or an operand with
  flag 1) and the stack object, missing as native errors 0x17 and
  0x15. The result is the route slots' direction (`0x4520e9`), plus 8,
  modulo 8. Global 34 aliases the destination as in Random Number. A
  destination with no word behind it (a literal, or a motive) fails
  loudly, because the original writes through a null pointer there.
- **The positions go in the native order.** The original passes +0xf4
  first, which is our position's second coordinate, as the slot code
  already knew (`pos = [target[1], target[0]]`). A first port that
  passed ours in order matched every object for 117 more ticks but
  left the lot stream one draw behind from tick 467 on. With the wrong
  direction, person 21's tree 0x1017 found object 126 beside door 93,
  so tree 0x100a skipped its Random Number.
- **How it was found.** A new capture of the original with a write
  watch on the active random state (`0x6e54ac`, `--watch`) logged each
  draw with its stack. Tick 467 had one, from Random Number
  (`0x451edd`) in person 21's thread. Our trace showed the tree that
  draws it is reached only when 0x1017 returns false. The capture
  matched the earlier one tick for tick, which also shows the
  original's runs are repeatable here.
- **K = 583.** Ticks 467 to 583 equal the capture, the three random
  streams included, but for the known last-place head fields. Tick 584
  stops at Snap (opcode 46).
- **What would reopen it:** a Get Direction To with a destination or
  object scope the Goth run has not reached.

## 2026-11-14: A blocked step, and K = 466

Tick 437 of the Goth house stopped where person 16's walk found its
next step blocked (`0x58f682`). Porting that led through two more
route steps in tick 438.

- **A blocked step ends the route blocked.** The walk checks that the
  person fits where the step ends (virtual function 0x54). If not
  (`0x58eecb`), it writes 3 to the route's end (+0x188; `esi` holds 3
  from `0x58edeb`) and goes straight to the update's tail (`0x58f688`).
  The step (+0x294, +0x298) is already written; the person and the
  animator's position stay where they were. That holds for the end of
  a path's extra step too. The stand's earlier writes stay (the
  destination's first two coordinates, the near-end and finishing
  flags, the state and the progress). Only the final copy of the whole
  position into the destination is skipped, and the state does not run
  again, so the standing animator never turns the 3 into 2. The capture's animator holds 3 after tick
  437.
- **Step 10 tries again.** A route whose animator stopped short of the
  path's end (0x589d47 gives 0, as it does for an end of 3) stops the
  animator (0x589e2d). If that fails, the run ends. Otherwise the next
  step is 6 (`0x45b371`). This was ported as a loud failure before.
- **Step 16 reads the context's slot, not a null one.** The route
  steps function reuses its slot argument (`[ebp+0xc]`): at `0x45aec6`
  every run sets it to the context's copy of the slot record
  (0x5c354f returns context +0xc). Giving up reads that copy's flag
  0x4000. The earlier entry's "null slot record on a later run" was
  wrong, and the loud failure is gone. A slot the route may end near
  still fails loudly (the failure feedback tree, `0x45af2a`).
- **Tick 438.** Person 16's route stops short, re-plans and finds no
  way, finds nobody in the way, and gives up. Word 62 takes the
  context's result, 13, left by the planning. The walk then goes on to
  door 87. The capture holds the same frames and word 62.
- **K = 466.** Ticks 437 to 466 equal the capture, but for the known
  last-place head fields. Tick 467 stops at Get Direction To (opcode
  12), not ported yet.

Reopen if a capture shows a blocked step that moves the person, or a
step-16 slot that differs from the context's copy.

## 2026-11-14: People in the way in the person vector's order, and K = 436

Tick 418 of the Goth house stopped where a route asked who is in its
way with two candidates (`0x45bb04`). We failed loudly there because
the lot's list of people was thought not to be modeled.

- **The list is the person vector.** `0x45ba28` walks
  `[person +0xdc] +0x84`, the same vector Set To Next's people modes
  walk (`0x44efb4`, `0x44f0d4`). The world keeps it as `World::people`:
  the loaded people in registration order, then each person made since.
  So the candidates are taken in that order, and any number of them is
  allowed. Steps 2 and 3 of the search find the first match in that
  order, as the original's static candidate vector does.
- **The branch into the route is the original's.** Our run takes door
  52's tree 4105 to Go To Routing Slot because the door's graphic (data
  word 0) is 4. The capture's door holds 4 as well, so the original's
  run took the same branch.
- **Tick 418 itself.** The candidates are persons 18 and 16. Person 18
  has a way but is not asked, because 18's last route context waits on
  person 21. Person 16 has no way. Nobody is in the way, so here the
  order does not change the outcome.
- **K = 436.** The state after ticks 418 to 436 equals the capture, but
  for the known last-place head fields. Tick 437 stops where person 16's
  animator finds its next step blocked (`0x58f682`).

Reopen if a capture shows people in the way taken in another order than
the person vector's.

## 2026-11-14: Run Functional Tree, and K = 417

Tick 417 of the Goth house stopped on Run Functional Tree (opcode 20),
node 7 of person 21's bookcase tree (0x1014). The handler is `0x44f681`.
It is reached through the general primitive table for every object; the
person's dispatcher does not take opcode 20.

- **The function table** is the jump table at `0x44f76a`:
  - functions 0 to 7 name callbacks 0x12 to 0x19;
  - 8 names 0x1a, 9 names 0x1b, 10 names 0x0e, 11 names 0x1c;
  - 12 names 0x10, 13 names 0x11, 14 names 0x1d.

  Any other function is false; the comparison is unsigned, so a function
  below 0 is false too. Function 8 asks the same person check as Find
  Best Object for Function (`may_ask_function_8`). Unlike that
  primitive's table, functions 4, 5 and 9 have callbacks here.
- **The action** is the first word of the callback's entry (`0x455fa9`),
  and 0 past the table's end. An action of 0 is false.
- **The push** is the running object's virtual function 0x34, the push
  Run Tree By Name's mode 2 makes: the running object is woken, the tree
  of the stack object's type gets the running object's temps (a null
  arguments pointer, `0x47484a`), and a person starts using the stack
  object (`0x45e59b`).
- **The flag** (byte 2, bit 0) goes into the person's use. A set flag
  also writes person +0xa54, which is not modeled, so it fails loudly.
- **The return branches like a call.** The handler returns 3 without
  writing the frame's state (`0x44f75c`), while Run Tree By Name writes
  1 first (`0x44bc3d`). So the return takes the branch of the tree's
  result, as a subroutine's does. This contradicts the legacy note that
  Run Functional Tree resumes; the interpreter comment is corrected.
- **Error codes.** A missing stack object is native error 0x15, thrown
  before the function is read. A tree the type lacks fails loudly here;
  the original's push would give false.
- **K = 417.** After the tick, person 21's frames and object uses equal
  the capture's: 174 at depth 7, 52 at depth 8.
- **The verifier compares the carry layer now.** The capture shows the
  carry slot filled, as ours is. The layer's own fields are on a page
  the capture lacks, so they stay unverified.

Reopen if a capture shows a functional tree's return resuming the
primitive instead of branching.

## 2026-11-14: Animate Sim on the carry slot

Tick 417 of the Goth house stopped on an Animate Sim with flag 0x10
alone: person 21's bookcase tree (0x1014, node 19) plays a clip on the
carry slot (animator +0x13c). The original's path is `0x589683`.

- **The animator keeps the carry layer** (`Animator::carry`, snapshot
  format 21). Like the foreground layer, its update hands timed events
  to the animator, which scripts can read (the event queue, the hands).
  The base layer stays a flag because its clips have no events.
- **The path, as the disassembly shows it.** Clip 0 clears the slot
  (`0x592878`) and sets the carry mode to 0. Any other clip is looked up
  as for the foreground (`0x5893b3`, which reads the appearance again
  when stale). Flag 0x80 slows the clock. The clip starts in the slot
  (`0x592726`), backward with flag 0x02. The carry mode becomes 6, and
  the layer is set to repeat (mode 1) at priority 10 (`0x59867f`). Both
  ways the carry selection becomes -1 and the true branch is taken, with
  no yield and no events followed.
- **The skin's list order is kept** (`Animator::carry_first`). The
  skin's update goes through its layers in list order (`0x5992b3`), so
  whichever layer comes first has its timed events handled first.
  Every layer is made at priority 1 and put in before the first layer
  whose priority is above 1 (`0x599128`); its own priority is set
  afterwards. So a carry layer goes before a foreground layer of raised
  priority (6) and after one of priority 1. A new foreground layer
  always goes before the carry layer (10).
- **Removal is immediate.** `0x592878` fades a layer out only when byte
  0x6f6bd0 is set, which holds 0 in a running game. So the reset, a
  pickup clip (flags 0x11) and clip 0 each remove the carry layer at
  once.
- **Still failing loudly:**
  - a clip the lookup does not find (the original reports it and goes
    on);
  - the head's pose while a carry layer plays (its bones could share the
    foreground's path to the head);
  - the base slot (flag 0x01 alone);
  - the animator's own carry clip (0x26).
- **Unverified:** the carry layer's fields against a capture. The tick
  stops before it ends, at Run Functional Tree.

Reopen if a capture shows the carry layer elsewhere in the skin's list,
or a carry clip that is not found and still plays.

## 2026-11-14: Two more cameras compared with the original

The original was shot at two more cameras, walls up, after the load's
notices were answered and the game was paused. Each was compared with
our drawing of the world after one tick of `sims boot`, with the
camera's origin fitted by search. The screenshots stay private.

**Medium zoom, rotation 0** (one zoom step in, then panned; origin
[463, -496]). Exact matches:

- Whole screen: 76.4% (72.2% of the pixels that are not grass).
- Open grass (x 520 to 780, y 380 to 480): 96.0%. The grass rule read
  at far zoom holds at medium zoom.
- The house (x 0 to 720, y 0 to 390): 74.1%. Walls, wallpaper and the
  wood trim match pixel for pixel where they were checked.

What differs is the objects. Doors, windows, shrubs, the bench and the
stairs match only about half their pixels. A door shows both kinds of
difference: its frame's left edge is two pixels wider in the original
(ours is hidden behind the wall, which is in front of it by depth), and
its glass sits one pixel further right. So at medium zoom both the
depth of objects set into walls and the placement of some draw-group
parts are off by a pixel or two. Neither is explained yet. The fence,
drawn as an object too, matches 97.6%, so this is not a shift of
every object.

**Far zoom, rotation 3** (one turn of the panel's right-hand rotate
button from rotation 0; origin [575, -203]). Exact matches:

- Non-grass pixels: 55.2%. The house (x 230 to 480, y 200 to 400):
  55.2%, most of the loss being grass inside the box. Walls, trim,
  floors and furniture line up.
- Open grass (x 560 to 760, y 440 to 540): 12.7%. **The grass does not
  match at this rotation.** The shading has the same shape, but its
  dark streaks sit in other places, and the blades differ. Counting the
  vertex jitter and the blade map from view tile 0, or from either
  rotated lot corner, instead of from the lot's least view corner,
  reaches only 16%. The cause is **unverified**: the shading, the
  jitter's coordinates or the blade map may each turn differently in
  the original. Reading how it turns the heights (0x479567) and the sun
  (0x4a4551) is the next step.

**Seen at both cameras:** lamps are missing. The floor lamp in the
upstairs-left room and the kitchen counter's lamp (also missing at the
first camera) are drawn by the original and not by us. Lamps may be
drawn through a sprite their scripts switch on, as other dynamic
sprites are; that is unverified.

**Unverified:** near zoom, rotations 1 and 2, and the cutaway at these
cameras (the screenshot at medium zoom had no wall cut, so it adds
nothing to the cutaway check).

## 2026-11-14: Wood trim along the tops of walls

The original lays a strip of wood trim along the top of every standing
wall and every cut-down wall. It is now drawn, read from its code
(0x53c227, the corner drawing) and the old prototype's corner pass
(`legacy/crates/sims-sim/src/lot_presenter/walls/construction.rs`).

- **One sprite per corner.** Each corner a wall touches gets a mask of
  the directions its walls leave it in: 1, 2, 4 and 8 for view -y, -x,
  +y and +x, 0x10 to 0x80 for the diagonals. A table at 0x6cd468 turns
  the mask into a frame; masks it has no frame for draw nothing.
  Standing walls' trim comes from SPR# 4096 to 4098 (by zoom) at the
  storey's height. Cut walls' trim comes from SPR# 5120 to 5122 at the cut
  height (6, 12 or 23 pixels), except along a diagonal through the
  corner (bits 0x10, 0x40), which takes the standing set.
- **Which walls.** Fences take none. A cut wall in a door-like style (3,
  5, 6, 15, 23) takes none. A cut wall meeting a standing wall gets a cap
  instead, which is not drawn yet.
- **Depth.** Each trim pixel takes the depth of the wall top under it.
  The original's own depth sprites for the trim (SPR# 6144 to 6146) are
  not read.
- **Light.** The trim is drawn at full light. The original's trim
  pixels are the sprite's own colours, even in the kitchen, which sits
  at half light.
- **Position: fitted.** The code puts the sprite half a tile left of
  the corner and one more pixel left. Here that extra pixel puts every
  trim one pixel left of the screenshot, so it is dropped; our corner
  point sits one pixel left of the original's.

**Result at the compared camera** (far zoom, rotation 0, floor 1, the
world after one tick of `sims boot`). Exact matches:

- Walls up, box around the house (x 120 to 500, y 100 to 320): 72.3%
  before, 75.7% now. Whole screen: 83.8% to 84.4%.
- Cutaway (the settled screenshot): box 67.1% to 69.0%; whole screen
  82.4% to 82.8%.

**What still differs:** the caps where a cut wall meets a standing one.
The original draws there a narrow upright slice of the wall's own
pattern with trim down its edge (0x53c6b7 onward, using the descriptors
at corner +0x28 and +0x3c). The extra cut-trim pieces the original adds
at a diagonal corner with bits 8 or 4 are also missing.

**Unverified:** other zooms and rotations, the cut trim on diagonal
walls, and the trim on terrain that is not flat.

## 2026-11-14: The cutaway cuts the rooms the original is looking at

Until now the cutaway cut every wall with a room on the camera's side.
The original cuts far fewer, and the rule is now read from its code (the
memory capture, as for the grass), following the old prototype's ports
in `legacy/crates/sims-sim/src/lot_presenter/`.

- **Which rooms.** The view keeps a list of three rooms, newest first,
  that never holds a room twice (0x49e923; its size is set at 0x4a6d96).
  Each pointer move in the cutaway adds the room under the pointer, then
  the first other room straight below it on screen, looked for every
  half tile down to half a storey (0x4ab7ee). The pointer is placed on
  the terrain as drawn, by the original's tile search (0x4a955d, 0x4a97c3).
  Off the lot, or on its outer ring of tiles, the mask is emptied
  (0x4ac07b) but the list is kept for the pointer's return. A change of
  floor empties the list (0x4a80c7). In live mode the selected Sim's room
  is added too, when the Sim is on the floor shown (0x4ab992). That is the
  room the simulation keeps for the Sim, which on a split tile is not the
  tile's. So the choice depends on the pointer and on who is selected: it
  is input, not lot state. `Camera::focus` carries it, and `Focus::point`
  does the pointer's part.
- **The mask.** Those rooms become a mask of view tiles (0x468203). Each
  of a room's tiles is in it, and so is every tile below one of them on
  screen whose standing walls would overlap it. An outdoor room adds no
  tiles of its own. An outdoor selected Sim adds a square of up to 4 by
  4 tiles towards the camera. If any of the rooms is outdoors, the
  pointer adds a 4 by 4 square: from the tile four tiles' height below it
  on screen, four tiles back up.
- **Which walls.** A wall is cut where the mask holds the tiles on both
  sides of it (0x54059a; diagonals have their own offsets). Fences and
  the like (styles 2, 12, 13, 14, 255) are never cut. Only the top floor
  shown is cut; the floors below stand (0x4a8895). Walls-down now follows
  the same floor rule.
- **Objects.** An object that hides with the walls hides when the mask
  holds its tile and the tile its facing side looks onto (0x4aa446). The
  mark is flag 0x400 of data word 8. Doors and windows set it in their
  own scripts as the lot loads, so a house read without a load lacks it.
  There, objects that cut their shape into a wall still hide, as before.
- **Cut doors.** A cut wall of style 3, 5, 6, 15, 22 or 23 draws a plain
  wall's stub. The original marks those styles with bit 8 when it cuts
  them. Doors stand in style 3 walls, whose own cut sprite is an outline.

**Result at the compared camera** (far zoom, rotation 0, floor 1, the
settled cutaway screenshot). Which rooms the original had in its list
cannot be read from the screenshot. The best fit is the kitchen (room 1)
from the pointer, plus the bedroom, where the selected Sim stands.
Adding the living room or the bathroom lowers the match. The foyer and
the bathroom now stand, as in the original. Exact matches:

- Box around the house (x 120 to 500, y 100 to 320): 61.0% before.
  65.6% now, with the house read as before; 67.1% with the world after
  one tick of `sims boot`, whose doors and windows carry their flag.
- Whole screen: 81.2% before, 82.0% now, and 82.4% after a boot.

**What still differs:** the trim along the top of cut walls (since
added; see the entry above), the Sims,
and the light on a few walls. The bedroom's west wall, with its door,
is cut in the original but stands here. The hall behind it, under the
stairs, is outdoors in our rooms, and no mask from an indoor room
reaches west of a room. So either the original counts that hall as a
room, or its list held something else.

**Unverified:**

- Rotations, other zooms and the upper floor. The tests check that a
  looked-at room loses exactly its front walls at every rotation.
- Diagonal walls. The offsets follow the old port, and the house has
  none in a cut room.
- The pointer's 4 by 4 square, and the terrain-aware pointer, against
  the original: this house's lot is flat where it was compared.
- Whether the list also empties on a rotation or a zoom. Other callers
  of the clear (0x4a81a1, 0x4a8517, 0x4a85fa, 0x4a86c2) were not read.

## 2026-11-14: The grass is drawn as the original draws it

The speckled grass is gone. The ground is now drawn the way the original
draws it, read from its code. The installed game is encrypted, so the code
was read from a memory capture of the running game in the harness's
private cache; nothing from it is in the repository. The addresses below
are in that image.

- **Palette** (0x49c8d3, 0x49e6cf). 32 grass colors run from brown to
  green. Each has 128 shades from light to dark. The light end blends the
  light brown 0x9d7541 into the light green 0x50743b; the dark end blends
  0x513c12 into 0x0e3508. A shade is a 4 by 2 block: two of its pixels
  are 8 and 6 shades lighter, and the second row is mirrored. Channels
  are truncated, then cut to 16-bit color.
- **Shade at each vertex** (0x4a4994, 0x4a4783, 0x4c47f7). The light on
  each of the six triangles around a vertex is the squared cosine between
  the sun and the slope. A tile is 2√6 height units wide (0x434df6). The
  sun stands π/10 from the zenith at rotation 0. Rotations 1 to 3 use 2,
  3 and 2 tenths. The mean becomes a shade, 127 − ⌊127·mean⌋. A jitter of
  up to two shades is added from the game's generator, seeded 0x40de9. A
  wall on the tile's far edge darkens that edge by 64 shades.
- **Grass color** (0x49cd43). Each corner averages the grass values of
  its four tiles and scales them to 0 to 31.
- **Columns** (0x49dacd, 0x49da21). A tile is drawn in four-pixel
  columns. Shade and color are interpolated down each column in 256ths.
- **Blades** (0x49dfbd, 0x49d79d, 0x49d8fb, 0x49cf9f). 4800 blades are
  scattered over a tile's shape from seed 0x4aa47b44. Each has a rank, a
  column, a depth down it, a shade 0 to 60 darker than the ground and a
  height (one pixel at the two far zooms, three or four near). A tile
  draws the blades whose ranks fall in a window. The window's size
  follows the tile's grass value: 500 to 1200 blades, a quarter of that
  at far zoom, none on a floor. Its start comes from a random map seeded
  0x4aa47b44.
- **The lot's size matters.** The blade map and the jitter loop both run
  over the lot's own size, not the arrays' 64 (0x49c9c7, 0x4a4a43). The
  Goth house is 52 tiles. That size sets which random draw each tile and
  vertex gets. `open_house` now applies the size the house asks for in
  global 23, as the load's resize does (`Interpreter::resize`). Before
  this, fitting single tiles found the blade counts right and the starts
  wrong. The starts matched the map at 52 entries a row. Fixing the
  jitter loop took the grass box from 84% to 100%.
- **Edges.** Ground is drawn only from tile 1 to the lot's size less 2
  (0x49cecc). Beyond the lot the window shows the original's grey, 16-bit
  (14, 28, 14), which is now `BACKGROUND`.

**Result at the compared camera** (far zoom, rotation 0, floor 1, 800x600,
the screenshots of the lighting entry below). Exact matches:

- In a box of open grass (x 530 to 640, y 335 to 365): 0% to 100%,
  in both wall modes.
- The whole screen: 18.1% to 83.6% with walls up, and 16.1% to 81.2%
  with walls cut away.
- The box around the house (x 120 to 500, y 100 to 320): 72.2% with
  walls up and 61.0% cut away. It was 37.5% and 26.3%.

Nearly every open grass tile now matches at least 90%. **What still
differs:** the house and its objects (lamp pools, the cut-wall cap,
some gravestone sprites), Sims, which are not drawn, and the game's
control panel in the bottom-left corner.

**Unverified:**

- Other zooms (taller blades near) and rotations. In particular, whether
  the blade map and the jitter loop count from the lot's least corner in
  the view, as here; the pool test at 0x47c16d suggests view
  coordinates. At rotation 0 that corner is 0 either way.
- Depth on sloped ground. A tile's ground takes the depth of its lowest
  corner. Following the slope would hide sprites behind hills better, but
  floors take their top corner's elevation, and the grass then covered a
  floor's edge where sloped ground meets it (0.1% fewer exact pixels).
  One depth model for floors and ground would settle it.
- Grass on a pool, which the original leaves bare. Pools are not looked
  at.
- A floor tile split by a diagonal wall. The original keeps the blade
  count of whichever tile it drew before; here it counts by its own
  grass.
- Seasons, which swap the blade-height tables (0x6c629c, 0x6c62a8).
- A flag at 0x8d that blanks the colors of tiles on the lot's edge. It
  is off in this capture and not modelled.

## 2026-11-14: Rooms darken by their light in the lot drawing

Floors, walls, terrain and objects are now drawn darker in a dim room, as
the original draws them.

- **Which light.** Each room's shown light comes from the simulation
  (`Room::shown_light`, read only). As in the original (0x46772d), it
  becomes a byte, `light * 255` rounded half up. Drawing reads the byte
  back as a fraction and keeps it between 0.5 and 1; that clamp is the
  old prototype's reading of the drawing context
  (`legacy/crates/sims-vm/.../draw/context/light.rs`). So even an unlit
  room shows at half brightness. Outdoor rooms the light pass never
  counted are drawn at full light.
- **Where it is read.** A floor half takes the light of the room under
  it; a split tile's two triangles each take their own room's. A wall
  takes the room just beside its middle on the side that shows, so a
  split tile gives the triangle touching the wall. An object uses the
  room at its position. Terrain uses floor 0's room at the tile centre.
- **How it darkens a pixel: fitted, not read from code.** The original
  draws in 16-bit color. Dividing its pixels by ours in the Goth house
  gave a sharp peak at 0.517 indoors and 1.0 outdoors. The booted
  snapshot's indoor rooms all have light at most 0.127, so they sit at
  the 0.5 clamp. Fitting each 16-bit channel gave
  `(v * k + 255) >> 8` with k = light in 256ths for red and green and
  one more for blue. That reproduces 69% of the indoor pixels exactly.
  The other 31% are mostly one step off.
- **Result at the compared camera** (far zoom, rotation 0, floor 1,
  800x600). Exact matches among non-grass pixels:
  - walls cut away: 33.9% to 42.2%;
  - walls up: 39.7% to 47.3%.

  In the box around the house (x 120 to 500, y 100 to 320), exact
  matches rose from 6.8% to 26.3% with walls cut away and from 20.5% to
  37.5% with walls up.
- **What still differs.** The grass (ours is speckled; the original's is
  smooth and shaded) and the wood cap on cut-down walls. Some gravestone
  sprites differ, and Sims are not drawn. Some pixels in the house are
  lit between 0.6 and 0.9 of ours. They look like pools of lamp light,
  probably from the falloff tables in `lightMap.dat`; they are not
  modelled.
- **Unverified.** Every light level other than the 0.5 clamp and full
  light. Night outdoors. Whether the original lights a wall by the room
  on its visible side. Lamp pools. Any of these would reopen the formula.

## 2026-11-14: Find Best Object for Function

Tick 417 of the Goth house stopped on Find Best Object for Function
(opcode 14): person 21's bookcase tree (0x1014) asks for the best object
for function 8. The original's handler is `0x44f2bf`, reached through
the primitive table at `0x45246a`.

What was decided:

- **It is ported whole** (`find_best_object.rs`), every function and
  flag of the handler, not only function 8: the table at `0x44f641`
  (callback and score attribute per function), the room flags, the
  check trees, the origin a variable names, function 11's halved
  distances and function 15's category walk. Functions 4, 5 and 9 give
  false, as the original's table does.
- **The walk is registration order.** It follows the list at
  `[lot+0x30]` through each object's +0xe0, the list Set To Next mode 2
  walks, which is `World::order`.
- **The weight is a setting.** The 3 that multiplies the distance is
  the global tuning's "functional distance attenuation" (`0x6c4394`,
  written once at `0x4592c4` with a default of 3), read with the other
  autonomy settings (`autonomy::Constants::distance_attenuation`).
- **An entry's check tree is our callback `condition`.** The original
  reads offset 0 of an entry as the action, as its fallback from the
  definition's action words shows (`0x4561e0`), and offset 2 as the
  check tree. In the game's files the second field we parse is the long
  tree and the first the short "CT - ..." ones, so `condition` is the
  check tree.
- **A check tree that changes the object list traps** `Unsupported`,
  whatever it gives: the original walks the live list, and we walk a
  copy.
- **An unordered score counts as 1**, as the original's `fcom` and
  `jae` (`0x44f5ef`) give, and never replaces the best (`jbe`).
- **Function 8's person test** reads person data word 65 (+0x60e), the
  word Idle For Input reads; the original casts the running object to
  its person class first, and a non-person gives false.

**Unverified**: the primitive against a capture of its own. It picks
object 174 in tick 417, as the fourth capture shows; the tick then
stops on Animate Sim's carry slot (`0x589571`), so nothing after the
pick is compared yet.

What would reopen it: a tick whose pick differs, or a check tree that
edits the object list.

## 2026-11-14: The person vector, and K = 416

Tick 409 of the Goth house differed silently: person 18's waiting tree
(semi-global 8276, calling 8291) looked at person 16 where the original
looked at person 259. Tree 8291 walks the people with Set To Next mode
1, and for each one in its room but itself draws one of two, looking at
the first that draws 1. Ours walked the object registration list, where
person 259, made at run time, comes first: `[259, 21, 18, 16]`. The
random streams agreed after the tick, so both sides drew three times;
with the draws ours made (1 to look round, then 0, then 1), the
original's result needs person 16 before person 259.

The original's mode 1 (`0x44efb4`, found through the jump table at
`0x44f25f`) walks a vector of person pointers at
`[[person+0xdc]+0x84]`, not the object list; mode 12 (`0x44f0d4`) walks
the same one. So the world now keeps that vector.

What was decided:

- **`World::people` is the person vector** (snapshot format 20,
  `format_19_bytes` proves the older pins): the people a load reads, in
  registration order (as before, from the old prototype's reading of
  the load), each person the factory makes appended after it is
  constructed, and a removed object taken out. A name in it with no
  object, or an object that is not a person, is `Trap::Invalid`, not
  skipped.
- **Unverified**: the code that appends to the vector was not found (the
  person code passes it to `0x4258a3`, which does not use it), so the
  end is read from tick 409's draws, and when in the making the person
  joins is not known. No capture page holds the vector. Any order with
  person 16 before person 259 fits tick 409. Because of that, a walk of
  the vector while the factory is running a new person's scripts traps
  `Unsupported` rather than showing the person or hiding it. Two people
  made at run time are taken to join in the order they are made, as a
  vector's append gives.

What would reopen it: a capture or trace of the vector, or a tick where
two people made at run time are walked.

K goes from 408 to 416. Tick 417 stops loudly on Find Best Object for
Function (opcode 14), the next step.

## 2026-11-14: Live sound plays through cpal, behind a feature that is off by default

`cargo run -p sims-audio --features playback --example play -- <event>...`
plays sound events on the default output device. The player renders
22,050 Hz stereo blocks on the main thread into a ring of frames that is
allocated once. The device callback takes frames from the ring and
converts them to the device's rate (by linear interpolation) and sample
format; it allocates nothing. The device paces the rendering. A device
that stops calling back for two seconds, or reports that the stream is
gone, ends the program with an error. At the end, the program waits for
the device's latency and a tenth of a second more, so the tail plays.

- cpal is optional and only the `play` example needs it, so `cargo
  test` and CI build no audio library and need no device. This matters
  on Linux, where cpal needs ALSA's headers.
- It lives in an example, not the library: the game's real host will
  live in the app, and how it mixes sound with its frame loop is the
  app's choice.
- Run on macOS only (48 kHz, f32): the device opened and the events
  played with no errors or underruns. Nobody has listened to it yet.

Reopen when the app wants live sound: it can take this callback or use
its own.

## 2026-11-14: Message dialogs, and text blended as the original blends it

`sims_ui::dialog` draws the dialog a script's Dialog instruction shows.
It reads the record in `world.dialogs`, then the strings from `STR#` 301
of the file that owns the caller's running tree. The operand meanings
and default labels come from the old prototype
(`legacy/crates/sims-vm/src/dialog.rs` and
`legacy/crates/sims-vm/src/dialog/presentation.rs`). The labels are in
`GameData/UIText.iff` `STR#` 142: OK, Cancel, Yes, No. The icon is
`BMP_` 5000 + operand 1 in the same file, when the icon mode in operand 7
is 3.

The layout was measured on two screenshots of the original at 800×600
("Magic Town" and "Studio Town", the dialogs a tick after the Goths'
house loads):

- **Placement.** The dialog is centered, rounding down.
- **Spacing.** 21 pixels between the frame and what is inside it, and
  18 between the icon, title, text and buttons.
- **Title.** In `variablesans_14`. With an icon, it is centered between
  x 141 and the right margin.
- **Text.** In `variablesans_12`, with 23-pixel lines. Lines beside the
  124-pixel icon start after it, and lines below it use the full width.
- **Buttons.** Each is 100×33, from `WinBtn` (UIGraphics 17), centered
  21 pixels from the bottom. Each half of the bitmap goes to its end of
  the button, and the middle column fills the gap. The label is
  `variablesans_12`, 5 pixels down, centered with the original's
  rounding.
- **Width.** 207, 217, 227 and so on, the first that is at least twice
  the height. This fits both screenshots (567 and 557 wide, both 276
  high) and nothing else that was tried. It is **unverified** for other
  dialogs.
- **Colors.** The text is the panel's label color. Under the pointer the
  label turns white; cells 0, 2 and 3 of `WinBtn` are the same picture.

Compared pixel by pixel after reducing both to 16-bit color: the icon,
the title and the text match on both dialogs except for 5 pixels each. The
"Magic Town" button matches exactly. On "Studio Town" the pointer was over
the button, and 24 edge pixels of its white label differ: the white blend
is **unverified**.

No bitmap in the install matches the frame, so it is drawn from
measured edge colors with mitred corners. It differs on 273 of about
11,500 frame pixels, all at the corners, where the original blends a
highlight. It is an approximation.

Text is now blended the way the original blends it, for the control
panel as well. Before, it was a plain 8-bit blend, and every edge pixel
was off. The original works in 15-bit color. Coverage level `c` (1 to
14) gives the text weight `[3, 5, 7, 9, 11, 13, 15, 18, 20, 22, 24, 26,
29, 30][c - 1]` out of 32, and what is under it gets `32 - weight`, plus
1 at odd levels. The result rounds down, and green keeps 5 bits. About
20,000 edge pixels on the dialogs' dark blue fit only that weight table.
The extra 1 at odd levels was needed only by the button labels, over a
lighter blue, so it was fitted on two backgrounds. On the control
panel's screenshot, the change fixed 1,142 text pixels and broke one; the
panel's digests are re-pinned.

**Unverified:**
- The title the original shows when the title string is empty (the old
  prototype used the object's catalog name; this draws nothing).
- `$` tokens in dialog strings, which are left as written.
- The spacing between two or three buttons.
- The icon modes other than 3.
- Text entry dialogs and the special panels; these are not drawn, and
  `Message::read` returns None for them.
- A message too long for the window. It is cut after the last line that
  leaves the buttons on screen, so the dialog can always be answered.
- The width for three buttons. The dialog is made wide enough for the
  row of buttons, in the same steps of 10.

What would reopen it: the Yes/No "Help System" dialog (tick 2) or any
other dialog screenshot that disagrees.

## 2026-11-14: Change Suit/Accessory, and K = 408

Tick 368 of the Goth house stopped on Change Suit/Accessory (opcode 6,
`0x45d962`). It is ported in `primitives/change_suit.rs`, with the
animator's "dress" and "undress" (virtual functions 0xd8 and 0xdc,
`0x58d88f` and `0x58d919`) as functions on the person's accessory list,
which the world already keeps from the saved file. Our state then
equals the fourth capture through tick 408 in every field the verifier
compares, with two known exceptions: person 16's head target height is
one ULP low on some ticks, and its head turn a few ULP with it (as
before this step), and the capture holds
the accessory count, not the entry's name or binding. K goes from 367
to 408.

What was decided:

- **A suit no decoded avatar file names fails loudly; any other is
  taken to bind.** The original adds the entry, then binds its suit to
  the skeleton (`0x583559`), storing the name and binding only when that
  works; an entry that fails stays with an empty name. Read from the
  code, the bind fails only when the suit lookup (`0x5a044d`) does: the
  name is not in the avatar registry, or the registry's lazy load of it
  (`0x5a38da`) fails. The attach after it (`0x599427`, through
  `0x5954d8`) has no failure path. The content loader now reads every
  avatar file's skeletons and suits record by record, as the original's
  catalog reader does (so a file holding animations still gives its
  suits), and keeps the suit names (`People::suits`, lowered) and the
  files that stopped decoding (`People::suit_errors`). Putting on a
  name outside that set is `Trap::Unsupported` before anything changes,
  naming any such files. **Unverified**: that the registry holds exactly
  these names (the install has no NDX index, so where it gets them is
  not read) and that its lazy load succeeds whenever ours decodes. The
  capture shows the entry count (one, from update 368), not the entry.
  Sol asked that every bind fail loudly until the registry is ported;
  rejected, since the code leaves only that registry question, and
  refusing would stop the one case the capture shows.
- **Only the words a branch reads must be known.** Suits 0 to 7 read no
  person data; others read the suit and age, and the variant only on
  the suit already worn. Writing the suit and variant makes them known.
  The age compare is signed (`0x45d9b4`), so a negative age is a
  child's.
- **Clip events "dress" and "undress" still fail loudly.** They call the
  same functions, but how the event's value becomes a name and binding
  (`0x5912fd`) is not read yet.
- **The variant draws use the active stream** (`0x46f457`, as Find Best
  Interaction's draws), so `Machine::draw` is now shared.

What would reopen it: a lot that puts on a suit no skeleton file
defines (it stops loudly now), or a capture page holding the entry.

The first difference is in tick 409, where person 18's route plays its
waiting tree and looks at another person than the original does. It is
silent, not a stop, and is the next step.

## 2026-11-14: A route asks a person in the way to move, and K = 367

In tick 367 of the Goth house, person 18's route finds person 21 in its
way. The route now tries again (step 6), asks person 21 to move, and
plays the asking animation (step 14). Our state after tick 367 equals
the fourth capture. Tick 368 stops loudly on Change Suit/Accessory. K
goes from 366 to 367.

Terms used here. The *asking person* routes; the *asked person* stands
in the way. A *marker* is the small object made where the asked person
stands; the asked person is given an action to walk to one of the
marker's slots.

What was decided:

- **Route steps 6, 14 and 16 are ported** (0x45b066, 0x45b475,
  0x45aef7), with the retry count (+0x58) and noted place (+0x60,
  +0x64) read at the start of each run of the steps and written back
  at its end (0x45b9a0), not put back as they were. Step 16 fails
  loudly where it would read the route's slot flags on a later run,
  because the original reads them through a null slot record there
  (0x45ad54 passes 0 after the first run). *Superseded by "A blocked
  step, and K = 466": every run reads the context's copy.*
- **The person in the way and the marker are ported** (0x45ba28,
  0x45bf2e), in `primitives/in_way.rs`. The lot's own list of people is
  not modeled, so more than one candidate fails loudly (0x45bb04): the
  order would matter. The marker's move slot is kept in a new map,
  `Routing::slots`, which Go To Routing Slot reads before the type's
  slot table; deleting an object drops its records.
- **Queueing an action is ported for the plain case** (0x45d20b): the
  next action number, then a priority insert; a full queue (8) takes
  nothing. Running at once, putting at the front and pushing queued
  actions out fail loudly. Taking an action out of a queue (0x45d5f8)
  calls each removed action's object with function 4 and then checks
  that the queue was not changed by those calls; the person running the
  action fails loudly.
- **Snapshot format 19** keeps the context's action number (+0x90) and
  the slot records. The format 18 pins still hold with them left out
  (`format_18_bytes`).
- **Slot flag 0x2000 adds a random part** to each place's score: one
  draw from the active stream, modulo 20, less 10 (0x483b02). The
  capture's 72 draws in tick 367 match one draw per place in range.
- **The slot search scores each place once.** The loop at 0x483a5d
  takes the place at the standing index or at the index after the
  seats, whichever weight is larger, never both. Our second pass over
  refused places was wrong; it never showed before, as classification
  has no other effect. This is a correction to an earlier reading.
- **Look Towards modes 2 and 3 are ported** (0x45a9c1): a route from the
  person's place to itself and then to the target (mirrored through the
  person in mode 3). Mode 1 reads the camera and fails loudly.

Not yet reached by a capture comparison: step 14's waiting tree, the
third and fourth retries, step 16 and the end-of-route release. They
are ported from the disassembly. The capture shows the waiting tree at
update 409 and the route ending at 489, so they are checked as K passes
those ticks.

The Sol review raised two findings, both rejected:
- *Function 4 should run after the action leaves the queue.* The
  original runs it with the queue intact (0x45d697 to 0x45d70b; see
  `release_action`).
- *Format 19 should still decode format 18.* Snapshots refuse any
  other format by design (`FORMAT`), as for formats 15 to 18. Saves
  with migrations are a separate format.

What would reopen it: a capture where a slot search with flag 0x2000
draws a different number of times, or a house with two people in a
route's way.

## 2026-11-14: The ambient scores play as the original's score player does

`crates/sims-audio/src/ambience.rs` plays the FSC scores (backyard,
beach, park, Magic Town). How the original's score player behaves was
read from its code in a memory image (read only, never written); none
of it was heard, since the harness runs the game with `-nosound`.

- A score is a grid of cells. A cursor moves over it: the row from a
  value the game computes from the camera, the column from the hour.
  On the score's beat the player picks cells near the cursor, weighted
  by each cell's chance, groups and exclusions, until enough are
  playing. A picked cell loads, starts on the next first beat, and
  frees its slot when its sound ends.
- The ambience modes (kind 52) and the ambience half of the music modes
  (kind 36) start and stop the scores as the original's dispatcher
  does. The music half of kind 36 is not ported and is still reported.
- Random draws come from the player's seeded generator, not the
  original's `rand`, so the picks differ from the original's but repeat
  for a seed.
- **Unverified:** how a cell's position maps to pan (taken as HIT pan
  `512 + 512x`); that the value 0x570c3e computes comes from the camera
  and that the second input is the hour; whether the sound system finds
  a cell's file outside the score's directory (the Magic Town score has
  50 such cells, which do not play here).
- Two of the original's quirks are kept: the beat clock wraps after
  about five hours of one score playing, and a beat can pick one cell
  past the score's limit. One guard is not the original's: a looping
  cell whose sound the mixer dropped frees its slot.
- **Left out:** fade in and out, the fast counter, the delay,
  quantise, stereo and third random fields, tempo changes and pausing.

The simulation has to provide the camera value and the hour (through
`Player::set_ambience_view`) for the cursor to move; until then the
cursor stays where the modes put it.

Reopen when a run with sound can be compared, or when the simulation
gives the cursor's inputs.

## 2026-11-14: The people viewer runs a booted lot

`people-viewer --boot` boots the house with `sims_vm::boot::Lot` (the
library form of `sims boot`) and runs it in the window: one tick every
50 milliseconds, because the animator clock moves 50 a tick at normal
speed. It catches up by at most 10 ticks per drawn frame, so a slow
tick slows the lot down instead of stalling the window. After each tick
the people are drawn from the lot's state as they are from a snapshot.
People a script makes while the lot runs are read from their objects
when they first appear, each on its own, so one person that cannot be
read (a type missing from the install) does not hide the others. `--answers` answers dialogs as `sims boot` does
(the Goths' house takes `--answers 3`). `--png` with `--at` runs `--at`
/ 50 ticks and then draws one frame.

When a step fails, the run ends there. The viewer prints the failure and
keeps the last state on screen. Today the Goths' lot stops at tick 367,
on a route that is not ported. That is the end of what the simulation
runs, not a fault in the drawing.

The viewer holds the lot and reads it only through `Lot::world`. The
library does not depend on `sims-vm`: only the viewer does (behind the
`viewer` feature), and so do the tests.

Checked: a gated test (`tests/lot.rs`) runs the Goths' lot to its end
and draws everyone placed every 30 ticks. At most four people are
placed at once (the family and a visitor). Each draws on its own,
covering part of the frame. Looked at by eye, the people move between
ticks. Unverified: none of this is compared with the original. Ticks
run at 50 ms, so the lot plays at normal speed; the slowed clock (42 a
tick) is not used. The camera still centres on the average of the
people, so on a whole lot some of them are off screen until it is zoomed
out.

## 2026-11-14: The standing Goths compared with the original

A screenshot of the original (House05, early morning, medium zoom) shows
the three Goths standing in their saved outfits. Ours were drawn standing
(the standing loop) at the same scale and compared by eye.

- **Cassandra** (person 16, a child) faces the viewer. Outfit, glasses,
  pigtails, proportions and pose match: arms hanging, feet together, the
  pocket protector on her left. Her height is 59 pixels in the original
  and about 61 in ours (within 4%). That also supports 3 model units per
  tile and the house view's 30-degree tilt.
- **Mortimer** (person 21) and **Bella** (person 18) are seen from
  behind. Outfits and proportions match. Mortimer holds a book: a carry
  layer can play over the base layer's loop, so this does not show
  which loop he plays. His carrying pose is not compared, and his base
  clip is unknown, so his standing loop is **not compared**.
- The phase of the original's loop is not known, so small differences in
  pose within the loop are not judged.

## 2026-11-14: How the original lights a person, and how far ours is from it

People are still drawn with our own light. This entry records what the
original does, read from its code, and what matching it would need.

**What the original does.** Before it draws a person (0x590c05), it
sets the lights (0x4b49c7), and puts them back afterwards (0x4b4cf5).
Direct3D's fixed-function lighting then adds the lights up per vertex.

- **A level from 0 to 1.** The draw reads a level from a lighting
  object it is given (0x4b4499): 1 when that object's byte +0x34 is set,
  otherwise its float +0x48, which a setter clamps (0x4b4525). The
  level is 1 when the person's object has bit 0 of its flags +0x114
  set. The person keeps its own copy (+0xe0). A new level starts a fade
  of 250 ms from the old one (0x5929ff, through the person's own
  method at vtable +0xe8). Reading it during the fade interpolates and
  clamps it to 0..1 (0x586dce, vtable +0xe4). The result goes to the
  lighting record (+0x20).
- **Ambient.** A grey of `round(255 × (low + (high − low) × level))`.
  `low` and `high` live in a static lighting object at 0x6f0150 and
  start at 0 and 1 (its constructor, 0x4b457c). With these defaults the
  grey is `round(255 × level)`. Whether anything changes them while the
  game runs was not checked (**unverified**).
- **The standard light.** One directional light from
  `GameData/Lights/stdlight.lgt`: white, intensity 1, direction
  (−0.644, −0.418, −0.641) in the files' frame. Each light of the record
  is turned by a matrix the game's view object gives (0x415711, +0x130)
  before it is set. Its strength is the same level.
- **Room lamps.** For each light of the room the person stands in, within
  a squared distance of 400 (20 units; which units these positions use is
  not pinned down), a directional light goes from the lamp towards the
  person. It is at full strength up to a squared distance of 225, then
  `1 / ((d² − 225) / 2 + 1)`. Its colour is (1.0, 0.984, 0.819).

**What a screenshot shows.** In two screenshots of the original, a
quarter turn of the view apart, a person facing the viewer is brighter
on the viewer's left. They are different people facing different ways,
so this only suggests that the light turns with the view.

**What we do.** One light fixed in the lot, from above and the viewer's
right (`raster::LIGHT`), with shade `0.45 + 0.55 × max(0, n · l)`,
where `n` is the vertex's normal and `l` the unit vector towards the
light. There is no level, no lamps and no fade.

**What matching needs.**

- The level. The simulation keeps each room's light from 0 to 1
  (`Room::light`, and which room a tile is in, on `World.rooms`), and the
  drawing can read it directly. That the original's level is the room's
  light is **unverified**: the lighting object passed to the draw, and
  what writes its +0x48, are not read yet.
- Which frame the view matrix (+0x130) is in. If it is the camera's, the
  light turns with the view, as the screenshots suggest.
- The materials' ambient and diffuse colours, which are not read.
- The room's lamps: their positions, from the simulation's objects.

**Stand-ins kept.** The base layer's clip and phase are not simulation
state yet. The animator only records whether the base slot holds a
layer (`Animator::base`). Which clip the original puts there needs more
reverse engineering on the simulation side. Until then the clip comes
from the saved person record and the phase from the animator's clock.

## 2026-11-14: Sound event kinds follow the original's dispatcher

The player now acts on an event by its kind, as the original's event
dispatcher does (read from its code in a memory image, never written).
Each event gives a track and three arguments; an argument the event
leaves at 0 takes the caller's value, which is the object for the
first. So a fade event, which never gives a speed in the install, fades
at the object's number, or at 100 for no object. That is what the code
does, odd as it looks.

Ported: play (1), fade (18), `radio_next_song` (19), kill all (20),
kill an object's instances (23), turn on a television or radio (30),
sequence a hit list (53) and kill every object's sounds (105). Kinds
whose case does nothing for sound are ignored. Not ported, and counted
in a gated test: music modes (36, 21 events), ambience modes (52, 5
events, the ambient scores), the piano (43), pausing (103) and
reloading (104). Twelve play events have an empty track and do nothing.

The radio, television and music hit lists come from directories, as the
original builds them when it starts: one sub list per subdirectory, one
patch per file. Their numbers are made up (from 2^30), since the
original's come from a counter. Files are taken in sorted order; the
original takes them as the file system lists them.

`SoundEvent::StopObject` is now kind 23: tracks the object was recorded
as playing for fade out at speed 100 and end, or end at once when
silent. Threads the object started without being recorded (tracks whose
arguments are not `kArgsIdVolPan`) keep running, as in the original.

MP3s are resampled to 22,050 Hz when loaded. The original sets every
sound buffer's rate from `pitch` (22,050 Hz at 3600), and its MP3s are
44.1 kHz, so it must hand DirectSound 22,050 Hz from its MP3 data
source, or play songs an octave low. Which it does is **unverified**.

Model limits: the original has one track object per track number,
shared by everyone playing it; this player runs a thread per play and
uses a track's newest thread where the original uses its one object.
When an object lets go of a track others still have, the original
re-evaluates the track (0x5746d9, not read); here that object's threads
fade out. A note whose thread has ended has no registers left to fade,
so it stops at once when its track is let go.

Reopen when a run with sound can be compared, when the music modes are
ported, or when the simulation needs pause.

## 2026-11-14: The live control panel, placed by measuring the original

`crates/sims-ui` draws the live-mode control panel from the original's
art (`UIGraphics`, ids from its control panel header) into a CPU frame.
The layout was taken from the old prototype
(`legacy/crates/sims-app/src/original_hud.rs`) and then checked against
a screenshot of the original at 800×600 (Goth house, live mode, Needs
open), by finding each bitmap in the screenshot.

What was checked:

- The cluster sits at the bottom left, 220 by 183. Every button's cell
  and position matched the screenshot exactly. Of its 28,244 pixels,
  255 differ, all on the translucent edge of the cluster or in the parts
  left out of the comparison (the clock line, the counter beside the
  mood face and the help button).
- The strip runs from x = 220 to the right edge, 100 high, its last
  column stretched to fill a wider window. Outside the meters, mood
  bars and portraits, 0 of 46,600 pixels differ.
- Portrait frames: 11 of 1,770 pixels differ, all under the activity
  icons, which are not drawn.
- The clock (font 8) and the funds (font 10) are drawn in green
  (565 value 0x2F2B) and match the screenshot's text. The Needs title,
  labels and meters match in place and color.

Three things in the prototype were wrong and are fixed here: the two
rotate buttons were swapped, the gadget sits 387 pixels from the right
edge (not 388), and at load the Live, normal speed, floor-down and
cutaway buttons show their selected cell and zoom-out its disabled cell.

The panel is placed against the bottom-left and bottom-right corners
at any window size, so a wider window shows more of the strip and the
lot. Only 800×600 was compared; how the original places the panel at
its larger sizes is **unverified**.

**Unverified or not drawn:**

- the counter beside the mood face (its source is not found; it may be
  an expansion pack's coins), and the help button;
- the activity icons over the portraits and the motive trend arrows;
- a family too large for one row: portraits wrap into a second row
  before the gadget (four columns at 800×600, so eight fit), and anyone
  past two rows is left out;
- which rotate button turns which way in the simulation's terms;
- the clock in the afternoon and the funds when negative.

Funds and the selected Sim come from the boot's household load (`sims
boot` steps 17 and 18), which the simulation does not offer as a
library call yet. The tests do the same two steps themselves; see the
roadmap section for what the simulation needs.

What would reopen it: a second screenshot (another window size, buy
mode, the afternoon) that disagrees.

## 2026-11-14: People are no longer drawn mirrored, and the lot lies the right way round

People were drawn as the mirror image of the original's. Now they are
not, and they stand on the lot the way the lot drawing has it.

**How the mirror was found.** Cassandra Goth (person 16) wears a shirt
with a pocket protector on one side. Two screenshots of the original
(House05, sitting on the porch bench and walking on the lawn, both seen
from the front) show it on her own left. Our drawing put it on her
right. So the image was mirrored.

**Where it came from.** The file conversion mirrored the X axis
(`[-x, y, z]`), to get a right-handed frame, as the old prototype's
renderer did. But our camera reads its frame as the original's renderer
does: x right, y up, z away from the viewer. So the mirror was never
undone. A scratch check showed our bones were exactly the X-mirror of
the original's own composition (0x59615b, ported as
`sims_sim::pose::absolute`), at rest and mid-clip.

**The fix.** Positions are kept as the files have them. A rotation
`[x, y, z, w]` becomes `[-x, -y, -z, w]`: the original keeps a turn with
x, y and z negated (its bone matrix, 0x595f32, negates them again). Each
bone now lands exactly where the original's composition puts it. A test
checks this on a chain of bones whose turns do not commute.

**The lot's layout.** Our placement put sim x on model X and sim y on
model Z. With the house view camera that drew sim +x up the screen. The
lot drawing's projection (0x4b3c88, `sims_render::camera`), which was
checked against the original's screen, draws sim +x right and down.
The original's model matrix (0x592132) puts sim y on X and sim x on Z,
and turns the skeleton by (direction + 6) eighths. Here the lot is that,
turned half a turn about the vertical, so that our camera draws it as
the lot drawing does:

- sim x becomes model −Z and sim y model −X (3 units per tile, as
  before);
- a person is turned by θ + π/2 about the vertical, for θ its direction
  times 45 degrees.

A person still faces (sin θ, −cos θ) in sim x and y, which is the way
the simulation walks them (the check of person 259 still holds).
Direction 3 now faces the viewer, not direction 7. A test checks the
screen directions of sim +x and +y and that direction 3 faces the
camera.

**What is verified and what is not.**

- Verified against the original: which side is which (the pocket), and
  that bones compose as the original's code does.
- Checked only against the lot drawing's projection, not against a
  person in a screenshot: that sim +x goes right and down. The lot
  drawing was itself compared with the original.
- **Unverified:** the pixel scale and offset of a person against the
  lot, and the camera's exact angle for the 3D people.

The pinned renders of the gated tests change, and so do the viewer's
demonstration and the tests' facing (direction 3).

## 2026-11-14: Booting a house is a library call: `sims_vm::boot::Lot`

The drawing tracks asked for a way to run a lot one tick at a time and
read its state in between. The load sequence and the tick loop were in
`sims boot`. They now live in `sims_vm::boot`:

- `Lot::load` runs the load and reports each step.
- `Lot::step` runs one lot update, then the house view's frame when the
  schedule (`Frames`) draws one.
- `Lot::world` reads the state.

A step that fails in its tick or a frame leaves the lot partway through
that work, so the lot halts: every later step fails and changes nothing.
The old CLI stopped there too. The motive constants are worked out at
the first step rather than at the load, so a load that runs no tick
doesn't need them, as before.

`sims boot` prints what the library does, and its output is unchanged
byte for byte: at K = 366 it gives the same hash, and at tick 367 the
same stop.

Why `sims-vm` and not a new crate: the boot needs the interpreter, the
linked program and the content loaders, which `sims-vm` already depends
on. A crate that depends on `sims-vm` reads the world through `&World`
only, so presentation never writes simulation state.

A frame that fails after its tick still reports that tick's update, as
the CLI listed it before. Commands from players will enter through
`Lot` when the command intake exists.

## 2026-11-14: File names hash as Windows-1252 bytes; lookups match the last path component (unverified)

The index places an archive member by the bytes of its name as the
archive stores them, not by the name decoded to UTF-8: the original's
file table hashes those bytes, so a decoded `café.iff` landed in the
wrong bucket and could reverse load order, and so which GUID definition
wins. A loose file's name is encoded to Windows-1252 before it is hashed
(the code page a Western Windows hands the game), so a loose file and a
member of one name stay one entry; a name that is not valid UTF-8 or
has a character the code page lacks keeps the file system's bytes.
**Unverified**: the install holds no name outside ASCII, and a
non-Western code page would differ.

`Index::named` compares a resource's last path component, so a GLOB link
to `SofaGlobals.iff` finds an archive member `sub\SofaGlobals.iff`. A
name that holds a directory still matches the whole member name, as
before, and `Resource::name` keeps the whole name for reporting. Which of several
such files the original picks stays unverified, as before.

A string table whose leading word is not one of the four version words
(0, -1, -2, -3) is a Mac `STR#` resource with a big-endian count, even
when that word would be negative read as a version: 128 strings start
`00 80`.

On the install, resource order, semi-global links, the census and the
gated digests are unchanged.

## 2026-11-14: Buy mode is driven by panel presses and confirmed by the mode, the registry and the budget

Scenarios can now use buy mode (`tools/harness/input.py`). There are three
new steps: `press X Y`, `expect-mode` and `expect-new-object`.

- **Presses on the game's panels work like dialog presses.** The pointer
  rests, moves over the button, then presses. A plain click was taken late
  or missed, as on dialogs.
- **What is checked.** The mode word is lot `+0x36`, read in the running
  game: 0 in live mode and 2 in buy mode. A purchase is confirmed by new IDs
  in the lot's registry and by the budget (lot `+0x90`). In buy mode the lot
  update still returns, with time not advancing, so the steps go on there.
- **Checked in the original, from a fresh boot.** The scenario:
  - entered buy mode (mode 2 at update 174);
  - pressed Seating and took the catalog's first entry, the $80 Werkbunnst
    chair;
  - pressed a free lawn spot;
  - went back to live mode (mode 0 at update 1078).

  At update 878 the registry held a new object, ID 262, at (344, 632) in
  sixteenths of a tile, level 1. The budget had gone from 2542 to 2462. The
  screenshots show the chair on the lawn and $2,462.
- **Unverified.**
  - A second new object, ID 292 at (-16, -16), off the lot, appeared by the
    same update. Whether the purchase made it is not checked.
  - Build mode's number is not known.
  - The catalog's contents depend on the panel's last state: after a fresh
    boot, Seating opens on chairs, and in an earlier session it showed beds.
    A press on an empty spot of the panel once sold an object. Scenarios
    must start from a known panel state (a fresh boot).

## 2026-11-14: Harness review fixes: the game's identity, watches and pages

A review of `tools/harness` found eight defects in the harness, all fixed
here, with tests that need no Docker (`tools/harness/test_capture.py`). A
finding in `scripts/check_repository.py` is fixed separately.

- **The game is pinned.** `guest/launch.sh` checks `Sims.exe` and
  `version.dll` against their SHA-256 digests and stops for any other build.
  It writes the verified digests to the share, and `capture.py` records them
  in `status.json` and as a `game_identity` event. Captures made before this,
  or of a game started some other way, show it as null. Reopen if the harness
  has to run another build.
- **Writes by other processes are not the game's.** QEMU's watchpoints are
  on virtual addresses, so another process writing the same address stops
  the guest too. Once the game's CR3 is known, such a write is a
  `foreign_watch` event, not a row in `watch.jsonl`. This includes a write
  found while stepping past a breakpoint. Before the first lot update the
  game's CR3 is not known, so early writes are still not checked.
- **Watches follow their pointers.** A watch through an object or a pointer
  is worked out again at every stop in the game. When an animator is
  replaced, the watchpoint moves to the new one (`watch_moved`), or waits if
  there is none (`watch_lost`). Writes between the replacement and that stop
  are missed. A watchpoint set at each change of the pointer itself would
  catch them; that is not done. Watches that land on the same bytes share
  one hardware watchpoint, which stays until the last of them leaves.
- **Pages.** An animator or layer that ends on a page boundary made the
  snapshot ask for the next page, which it had not read, and the capture
  ended with a `KeyError`. The last page is now that of the last byte.
  `pages.json.gz` no longer holds a page past the end of such a block.
- **`--dump 1` works.** Update 1's entry is the baseline's stop, which never
  dumped. A run taken over with `--resume` in the middle of a dump redoes it
  (a dump is complete once its `registers.json` is written).
- **The stub's checks hold under `python -O`.** The packet checksum and the
  allowlists that keep `rsp.py` and `qmp.py` read-only were asserts. They are
  now explicit checks.
- **Guest jobs.** `guest.py` reserves each job number by creating its file
  exclusively, so two processes no longer take the same number and lose a
  job. `click` and `move` pass on the guest job's exit code, and `click`
  stops if the pointer cannot be placed.

## 2026-11-14: Sound volume, pan, pitch and ducking, from the original's code

The sound player now applies volume, pan, pitch and ducking. Each
formula was read from the original's code in a memory image of the
running game (read only, never written); none was observed with sound
playing. The formulas are in `crates/sims-audio/src/levels.rs`. The
player's output is now stereo: `render_stereo`, with `render` as its mono
mix.

- **Volume** is `vol * extvol / 1024`, with `extvol` scaled by the
  track group's volume (the options' sliders, set by the host) and
  halved when `main_duckpri` is above the track's `duckpri`. The sound
  layer turns it into a DirectSound attenuation (0 dB at 1024, -40 dB
  just above 0, silent at 0), which we turn into a linear gain. A track
  starts with the HOT track's volume and duck priority.
- **Pan** is DirectSound's: 512 is the centre, and only the far channel
  is attenuated. No script in the install writes it.
- **Pitch** sets an absolute rate, `22050 * 2^((pitch - 3600) / 1200)`,
  whatever the sample's own rate. Every patch in the install is 22,050
  Hz, and the TV routines set pitch 2400 for 11,025 Hz files, which this
  plays at their own rate. Before, samples played at their own rate; a
  sample that is not 22,050 Hz now plays at the rate its pitch sets. The
  original adds a second track's pitch and subtracts 3600; which track
  that is was not found, and it is taken as 3600 (**unverified**).
- **Ducking.** `main_duckpri` (global 123) holds the largest request of
  any running thread; a thread's request goes when it ends, as the
  original's kill routine drops it.
- **Every register write sets the comparison flags**, as the original's
  write routine does. Branches after a write (such as
  `getsrcdatafield r; ifne`) now follow the value written. Before, they
  followed the last `cmp` or `test`.
- **`target` (53)** sends a thread's register reads and writes to the
  newest running thread of the track it names, globals included, and
  on through that thread's own `target`. A track with no running thread
  means the thread itself, as the original's null lookup does. The
  install's scripts use it to fade out and kill music and loops. How the
  original picks among several threads of one track, and what it does
  when the target has since ended, are not known.
- **Sequence groups hold track numbers, not thread IDs.** The ID the
  original stores in a slot is the one `target` looks up, and scripts
  pass track numbers to `target`. So two threads of one track no longer
  end each other. This corrects the previous entry.
- Levels are applied to notes once per block, after the threads run,
  and when a thread ends. The original applies them as registers
  change. 3D position, the
  second pitch and track priorities are left out.
- The exponentials use only basic arithmetic, so gains are the same on
  every platform. A track at full volume, centred, passes its samples
  through unchanged, so the phone ring and footstep still match their
  samples bit for bit.

Reopen when a run with sound can be compared, or the second pitch's
track is found.

## 2026-11-14: Directory listings are sorted, and paths are spelled as on disk

From the back review of main:

- **Listings.** `std::fs::read_dir` joins the clippy bans, since the
  order it lists names in depends on the filesystem. The few listings
  left sort what they read and say so beside an `allow`.
- **Spelling.** `sims_content::install::resolve` now always returns the
  spelling the directory listing gives, and a case-insensitive match
  takes the first name in sorted order. Before, a case-insensitive
  filesystem (macOS) returned the spelling asked for and a
  case-sensitive one the spelling on disk. Paths reach state (a
  skeleton's file), so the same install could hash differently on two
  machines. The people crate's lookup takes the first match in sorted
  order too.
- **Other bans.** The bans also cover `asinh`, `acosh` and `atanh` for
  `f32` and `f64`, and `Instant::elapsed` and `SystemTime::elapsed`.
- **Not banned: environment reads.** No library crate reads the
  environment. Only tests (the `SIMS_INSTALL` gate) and tool entry
  points do, and the install path enters as data. Reopen if a library
  crate needs one.

## 2026-11-14: Which catalog wins when two hold the same name

The original's code settles this for appearances and clips. Before
loading, it builds a list of search folders (0x5835ed, as the old
prototype traced it). It then lists the catalog files in those folders
(0x5c1a7d, as the old prototype ported it in
`legacy/crates/sims-sim/src/pose/clothing/catalog_files.rs`):
- every text catalog (`.cmx`) in every folder before any binary one
  (`.bcf`);
- each kind folder by folder;
- a file name already listed from an earlier folder is skipped.

It loads them in that order (0x59f844 for appearances, 0x5a0212 for
animations). For each record it looks the name up first. A name not yet
known is added. A name already loaded keeps its record, and the new one
is released (0x59f087, which calls 0x5954a5 for appearances and
0x59778a for clips). So **the first catalog loaded wins.**

For appearances the folder order is:
1. the game's own skins (`GameData/Skins`);
2. the shared expansion folders (`ExpansionShared/Skins` and others);
3. each expansion's `Skins`, `Animations` and `Accessories`, from the
   newest expansion back;
4. the first expansion.

An archive's members count as being in the archive's folder plus the
folder in the member's name. For example, `Deluxe.far`'s
`Skins\b200mafit_ctb.cmx.bcf` is in `deluxe/skins`.

`sims-people` now does the same. On first use it puts every catalog of
the install in that order, reads each, and keeps the first catalog of
each appearance name. A catalog it cannot read is skipped, as the
original goes on after a file it cannot read. Reading them all takes
under a second. This replaces the old lookup, which:
- tried the binary file named after the appearance first, then the text
  one;
- then took the first catalog in name order.

In the Complete Collection, one appearance is given differently by two
catalogs: `b200mafit_ctb`. It is in `GameData/Skins` as text and in
Deluxe's archive as binary, with different bindings. The original takes
the `GameData/Skins` one, and the old lookup took Deluxe's. A gated test
checks this one, and unit tests check the order and the first-wins rule
without the install.

What is still approximated or open:

- **Order within one folder: unverified.** It is the original's file
  map's, which is not ported; we use name order. No folder of the
  install holds two catalogs giving the same name.
- **Folders the original does not search.** Catalogs elsewhere (such as
  an expansion's object folder) are loaded after all the others. The
  original may not load them at all.
- **Clips.** The rule is the same, with `GameData/Animation` searched
  first. No clip name is defined by two files in the install, so our
  order (the first file in name order) never picks differently.
- **Same-named files other than catalogs: unverified.** 47 file names
  exist in two sources with different bytes: accessory textures such as
  `bottle.bmp`, some animation files shared by the third expansion and
  `ExpansionShared`, and a few meshes. Which copy the original reads
  depends on how it looks those files up, which is not ported. Its
  texture search lists the expansions before `GameData` (0x413ccc), the
  reverse of our order, and the expansions' copies of the accessory
  textures are larger. So the original may well prefer them. None of
  these files is drawn for the Goth family. We keep our order and mark
  it unverified.

## 2026-11-14: Scripted input reaches the game through xdotool, and is confirmed in memory

The harness can now drive the original from a scenario file while it
captures (`capture.py --scenario`, `tools/harness/input.py`). This is how
captures with player input are made, for our simulation to replay.

What was decided:

- **Input goes through xdotool in the guest, not QEMU.** QEMU's input
  devices do not reach the game: the guest's X server is Xvfb, which reads
  no device. A small guest job (`guest/input-agent.sh`) runs the commands
  the host drops in the share, polling every 50 ms; the generic job runner
  polls once a second. Guest memory is never written.
- **What was clicked is read from the pie menu, not trusted.** The
  picker tool's hovered object (picker `+0x2e`) lags the pointer by tens of
  lot updates under TCG, because the game takes pointer moves late. Once it
  named the bench while the pointer was over the porch floor, and the click
  opened "Go Here". So the hover reading only says when to press, and the
  pie menu's actions say what was clicked: a menu whose actions aim at
  another object is closed with Escape and the next aim point tried.
- **A chosen interaction is confirmed twice.** capture.py stops at
  `0x48ddcb`, the call in the pie menu's select handler (`0x48dd67`) that
  pushes the action on the person (`0x45d20b`, edi the person, esi the
  action), and logs it with the update it falls before or in. Then
  `expect-queued` waits for the person to hold that action, queued
  (`+0x820`) or running (`+0xa48`).
- **Dialogs are answered on the dialog stops.** A modal dialog holds the
  lot update it shows in, so a step that waits for update returns would
  never see it. `answer-dialog` presses once, as `answer.sh` does, and
  waits for the dialog's result stop.

**Checked against the original.** In a capture from a fresh boot
(2026-11-14), the scenario answered the five welcome notices and the Help
System dialog, aimed at bench 173 by projecting its position (hover
confirmed on the second aim point), opened a pie menu with the one entry
Sit (caller 21, target 173, routine 8213, priority 50), and chose it. The
push came before lot update 178. At the return of update 180 the person's
queue was already empty, and the full state after update 200 holds Sit on
173 as its active action, with uid 1. (That run's check read only the
queue; reading the running action too was added after it.) A second
scenario on the same running lot hit the bench at the same aim point, got
the same menu, recorded the push before update 168, and found Sit on 173
in person 21's queue (uid 7) at the return of update 168, behind its
running action. Screenshots showed the bench in the
person's queue. The memory layout used (picker, pie menu, people panel,
house view, queue) is listed in `input.py`; each part was read in the
running game.

**Unverified.**

- The projection was checked only in the starting view (zoom 1, no
  rotation), and it ignores the floor's height: the porch, on a
  foundation, is hit about 30 pixels above its projected point, which the
  aim points cover.
- Hovering is not free of side effects: `0x48c834` sets and clears
  highlight bits in the hovered object's flags (`+0x114`, through
  `0x4320da`). Whether those bits reach anything our simulation compares
  is not checked. A replay may need them, or a check that they are only
  drawn.
- Submenus, build and buy mode, and a missed dialog press (the step then
  waits for the dialog to be answered by hand) are not handled.
- Dialog button places are written in the scenario, not read from the
  dialog.

## 2026-11-14: The head follows its target, and K = 366

In tick 334 of the Goth house, person 16's head starts to follow what
it looks at. That is now ported, with what it needs: the places of the
head's points in the current pose, and the height of the ground. Ticks
335 to 366 then reach a few more mechanics, which are ported too. Our
state after every one of ticks 1 to 366 equals the fourth capture, but
for the last place of one head field. Tick 367 stops loudly. K goes
from 333 to 366.

Terms used here. A *heading* is an angle seen from above, in eighths of
a turn from 0 to 8. An *anchor* is a named point fixed to a bone, such
as the top of the head. The *current pose* is where every bone is when
the foreground clip is played at its present phase. A *joining entry*
is the interaction a person may start to take part in another person's
running interaction.

What was decided:

- **The head's follow, relax and settle are ported** (0x590768,
  0x5907c5), in `sims_sim::head`. The turn moves by at most 0.2 eighths
  per tick towards the difference of the two headings, and stops at 1.5
  either way, where word 42 changes. With the follow off it shrinks by
  a fifth each tick. Look Towards (frame step 6) now feeds it.
- **The headings are measured on the current pose** (0x5904d2). The
  pose is built when needed from the foreground layer, as the strides
  are, not kept. The HEAD bone is found by name and turned for the
  measurement only, then turned back (0x590b27). The person's model
  matrix (0x592132) turns the skeleton to its direction and lifts it to
  the height of the ground. Every add, subtract, multiply, divide and
  square root is rounded to single precision, as the game's FPU setting
  does.
- **The height of the ground follows the tile's corners** (0x478dfe).
  The tile is cut along its diagonal into two triangles; the height is
  taken on a plane through the triangle's corners. The original hands
  the function the person's rounded second coordinate first, then the
  first (0x592234, 0x592242). The old prototype's `Terrain::height`
  (legacy/crates/sims-sim/src/pose/transform.rs) has the same shape.
- **Snapshot format 18** keeps the head's fields (animator +0x210 to
  +0x224). The target is None until something writes it: the original
  leaves those words as memory held them. The pins taken under format
  17 still hold with them left out (`format_17_bytes`).
- **A walk can start with a clip** (animator state 6, 0x58e125). The
  person moves one clip's distance straight ahead, following the clip
  until its phase reaches 0.95.
- **Find Best Interaction scores joining entries** (0x41ee99, called at
  0x458aee). For a person whose running interaction may be joined
  (flag 0x2), the entry is that interaction's joining index on the same
  object. Its check tree runs with arguments 1, 1, 0, 0 instead of 1, 0,
  0, 0 (0x458372). When the joining interaction is missing, the
  original leaves the entry's target empty and 0x458319 returns before
  any effect, so we leave the entry out.
- **Run Tree By Name mode 1 is ported** (0x44ba26, 0x44bb4b). The tree
  runs to its end on the stack object's thread, with that object's
  temps (0x4749b5 with no arguments). No one is woken and no use is
  counted. Mode 0 still fails loudly.
- **The place test's refusal is told apart** (0x4337f6, read by
  0x48158c). A wall crossing answers 3. An object in the way answers 6
  when it is the route's own first object, else 5 for a person and 1
  for any other, and its id goes to the route's +0x8c. An exclusive
  clash with no object answers 6. Refusals the original marks with
  other codes answer 0. A refusal whose code we have not read (the
  wall rules of step 3) fails loudly at 0x48158c.
- **The planner gives up with no candidates** (0x482462), setting its
  result to 7 when it was 1. The route to a slot then clears its
  candidates and both indices (0x481898) and answers 0. Go To Routing
  Slot's step 6, which takes that answer, fails loudly at 0x45b066.

How it was checked:

- After every one of ticks 1 to 366, our state equals the fourth
  capture's, floats bit for bit, in the same fields as before, now with
  the head's fields (`tick_verify_long.py`, outside the repository).
- The height of the ground: from tick 356 on, person 259 stands on
  sloped tiles. The heights the capture implies at eleven points fit
  only the corner order above with the coordinates handed over in that
  order; the disassembly at 0x592234 confirms it.
- The second and third captures still agree where they hold a state.
- `the_goth_house_first_tick_matches_the_original` runs to tick 366,
  checks person 18's walk start in tick 335 and person 16's head target
  in tick 357, pins the states after ticks 333, 334 and 366, and checks
  that tick 367 stops at 0x45b066.
- The load comparison still gives its three known lines.
- New synthetic tests: the follow, relax and turn limits; the headings;
  the ground's height on each half of a tile; the walk start and its
  end at phase 0.95; a joining entry's check arguments; the refusal
  answers; and the planner giving up.

What did not match, and why:

- Person 16's head target height (+0x220) is one unit in the last place
  lower in ours after ticks 335, 336, 338 to 340, 342, 343 and 353. The
  height of the ground was checked exact on those ticks, so the
  difference is likely in the pose of the top of the head. The captures
  do not hold the bones, so it cannot be traced further now. Nothing in
  the simulation reads this field.
- Person data word 37 still comes from a candidate word the original
  never writes; it is held as unknown and not compared, as before.

Unverified:

- The sine, cosine and arc tangent come from libm in double precision.
  The original's FSIN, FCOS and FPATAN may round differently at some
  inputs; no difference was seen.
- That Run Tree By Name mode 1 answers false for an object whose thread
  is in an error state (0x4749b5 reads +0x28). No object here is in
  one.
- Which code the place test's wall rules (step 3) leave for 0x48158c.
  They fail loudly when reached.

## 2026-11-14: Archive reads are bounded; compressed FAR members are left out (unverified)

Archive readers in `sims-content` and the census now cap what they load,
with the frozen prototype's limits: a FAR directory of at most 64 MiB
(`far::MAX_DIRECTORY`) and a resource of at most 128 MiB
(`far::MAX_MEMBER`). A member is checked against its archive again when
it is read, because the file may have changed since indexing. The
census opens archives inside archives at most four deep, rejects an
archive whose member overlaps its header or directory (such a member can
be the archive itself), and stops decoding members of one file on disk
beyond four times its length plus 16 MiB; it reads an archive on disk a
member at a time. An archive member whose name is longer than 1024 bytes
or holds more than 32 directories is left out of the index with a
diagnostic, since each directory in a member's name is one the index
lists. The game's own files are far inside all of these limits (member
names reach 96 bytes and four directories), and the install census is
unchanged.

A FAR 1a member whose unpacked size differs from its stored size is
compressed. Decompression is not implemented, so such a member is left
out of the index with a diagnostic, as the prototype refused it, and the
rest of its archive mounts. In `GameData/Animation`, which holds only the
game's own files and is read outside the index, such a member fails the
load instead, so a clip or skeleton never goes missing silently.
**Unverified**: whether the original lists
such a member, and so whether it hides a later file of the same name.
The game's own archives hold none. Keeping both sizes on
`sims_content::Member` waits on `sims-people`, which builds that struct
itself and also reads FAR directories without these caps.

## 2026-11-14: People: transparent texels, shared edges, and people made after the load

A review of `sims-people` found five defects. They are fixed as follows.

- **Transparent texels.** A texel's alpha was ignored, so a texel a
  run-length bitmap skips was drawn opaque black and hid what was behind
  it. Now a texel with alpha 0 draws nothing and leaves the depth buffer
  (the nearest depth drawn at each pixel) alone. A texel with alpha 255
  is drawn and sets the depth. A texel in between is blended over the
  pixel and leaves the depth alone. The canvas holds straight colour (not
  multiplied by alpha), as a PNG does, so the blend weighs what is below
  by its own alpha. The game's skins are either opaque or
  fully transparent, so the in-between case only keeps other textures
  sane. What the original does with partial alpha is **unverified**.
- **Shared edges.** Testing the alpha fix showed a second fault. A pixel
  exactly on the edge two triangles share was drawn by both. Now each
  triangle owns only its top and left edges (the usual "top-left" fill
  rule). Which side of an edge a pixel is on is always worked out from
  the same end of the edge, so the two triangles get exactly opposite
  answers even with rounding. A pixel near a shared edge is then drawn
  by exactly one of them. This changed the pinned renders of the Goths:
  7 to 43 edge pixels each, out of 8,000 to 16,000. No
  pixel became empty or newly drawn, so none of the change comes from
  transparency. The new digests were checked that way and then pinned.
- **Bit-field bitmaps.** A bitmap of type `BI_BITFIELDS` was read as if
  its colour masks were the usual ones. Now its masks are applied, for 16
  and 32 bits per pixel. A bitmap with an empty colour mask is refused,
  and so is one whose header does not fit in the file or whose pixels
  start inside its header or masks.
- **People made after the load.** With a snapshot, the viewer only drew
  people who were in the saved house. Person 259, the travelling salesman
  a script makes during the boot, was never drawn. Now the viewer draws
  every person object the snapshot holds. A person whose record names no
  clothes is drawn in its type's default outfit: the first descriptor
  (before any `;`) of the body, hands and head rows of the type's body
  strings (the string table its object
  definition names). These are the rows the old prototype's port of the
  original's outfit choice (0x58a178) reads. Whether the original draws a
  newly made person in them is **unverified**; the simulation does not
  keep the outfit it chose.
- **Walk clip.** Pressing W took the first three letters of the base clip
  as the walk clip's prefix, and panicked on a shorter name. Now the
  prefix comes from the skeleton: `c2o` for a child, `a2o` otherwise.
- **Weight test.** The test of a clip's weight started and ended at the
  same pose, so it proved nothing. Now it blends towards a different pose
  and checks weights 0, 0.5 and 1.

## 2026-11-14: Sound player limits, and sequence groups from the original's code

A review of `sims-audio` found seven defects. The fixes:

- **Limits.** One block runs at most 100,000 instructions across all
  threads, and one thread at most 10,000. A thread started by
  `play_trk` first runs in the next block, so a track that starts itself
  cannot keep a block going, and the threads a full block does not reach
  run first in the next. At most 256 threads, 1,024 notes, 256 sequence
  groups and 1,000 recorded faults exist at once, and calls nest at most
  64 deep. A patch no table defines is not cached, since any number can
  name one. These numbers are ours. The original reports an error after
  101 instructions without a wait and carries on.
- **Arithmetic and sizes.** Mixing and interpolation use 64 bits. An XA
  header's claimed size is checked against what its data can hold before
  any memory is reserved. FSC sizes use checked multiplication. Hit-list
  files may expand to at most 100,000 entries (the largest in the
  install has 3,661); past that, parsing fails. Writing a WAV whose
  sizes do not fit 32 bits is an error, not an overflow.
- **Unported instructions fault.** `seqgroup_wait`, `add_to_group`,
  `remove_from_group` and `test1` to `test4` end their thread with a
  fault. The original does nothing for the first three, and no program in
  the install uses them; `test1` to `test4` are debug routines.
- **Sequence groups are ported, not faulted.** The review asked for
  `seqgroup_kill` and `seqgroup_return` to fault too, but 2,731 of the
  4,494 event definitions reach `seqgroup_kill` (most voices), so a fault
  would silence them. Instead they follow the original's code, read
  (never written) from a memory image of the running game: a group slot
  holds the ID of the thread that last claimed it.
  - `seqgroup_return` claims the group and ends its own thread if another
    held it.
  - `seqgroup_kill` claims it; another holder ends at once when its
    `extvol` is 0, and otherwise fades `extvol` toward `fade_dest` at
    speed 200 while the caller waits 2 ms and runs the instruction again.
  - `seqgroup_trkid` reads a group's holder.
  - Slots are not cleared when a thread ends, so a stale ID still makes
    a later `seqgroup_return` end. Only `StopAll` clears them here.
  - New threads start with the original's default register bank
    (`extvol` 1024, `fade_dest` 1, `pan` 512, `pitch` 3600 and others).
  - A fade adds in 64 bits, so it stops at its destination where the
    original's 32-bit sum could wrap past it.
  - **Unverified:** none of this was observed at run time, and how often
    the original updates fades is not known (here once per block). Volume
    is not applied yet, so a fade changes registers, not the sound.
- **Every definition is tested.** The install test runs each event
  definition as its own module defines it, not by first match. A
  separate test pins that 13 names are defined by more than one module
  (5 of them differently), and that the module whose name sorts first
  wins. How the original chooses is unknown.

Reopen when a run with sound shows different group or fade behaviour.

## 2026-11-14: Hidden objects, sprite world offsets and soft edges in the lot drawing

A review of `crates/sims-render` found four defects. All four are fixed,
with tests.

- **Hidden objects are not drawn.** An object whose data word 34 is not
  zero is hidden; the simulation (`HIDDEN` in `sims-vm`) and the old
  prototype skip it, and now the renderer does too. Checked against the
  original: the Goth house file hides 18 objects, and one of them, a
  table on the road, showed in our far-zoom render but is not in the
  original's screenshot. At that camera (far zoom, rotation 0, floor 1),
  exact matches among non-grass pixels rose from 33.6% to 33.9% with
  walls cut away and from 39.5% to 39.7% with walls up. Objects inside
  other objects are not drawn at their slots yet, so a hidden container
  does not hide its contents here.
- **A draw group sprite's world offset is kept.** Each sprite of a draw
  group (DGRP) carries a position relative to the object. It was
  dropped. Its units: x and y are sixteenths of a tile in the object's
  own frame, and z is height, where one unit is `2 << zoom` screen
  pixels (16 at the near zoom). The old prototype divided x and y by 16
  to get tiles (`legacy/crates/sims-render/src/lib.rs`).
  - z raises the sprite on screen by `z * (2 << zoom)` pixels, rounded
    by adding a half and truncating, as the original's catalog preview
    does (0x438ae4 to 0x438b58, read by the old prototype in
    `legacy/crates/sims-render/src/object_preview/software.rs`). That
    the lot view places sprites the same way is **unverified**.
  - x, y and z all move the depth the sprite is drawn at. x and y turn
    with the object's facing, a quarter turn per two steps (the old
    prototype's rotation). The preview leaves x and y out of the
    sprite's screen position, and so does this; that is **unverified**
    for the lot view too.
  - None of the groups the Goth house draws now has a nonzero offset,
    so no render changed, and nothing here has been compared with the
    original.
- **Soft edges write depth.** A sprite pixel that is partly transparent
  now writes its depth as well as blending its colour, as the old
  prototype's blitter did. Before, a farther pixel drawn later could
  show through a nearer soft edge. This changed several pinned renders
  slightly. The compared camera did not change at all.
- **Sprite positions cannot overflow.** Draw group offsets are any
  32-bit number. Placing and clipping a sprite now use wider or
  saturating arithmetic, so a sprite far off the screen draws nothing
  instead of panicking. An image whose pixel count does not match its
  size, or overflows, draws nothing.

What would reopen it: a camera where an object with a nonzero world
offset can be compared with the original, or a reading of the
original's blitter that treats soft edges differently.

## 2026-11-14: The lot viewer opens a window with winit and softbuffer

`crates/sims-viewer` is a small program that shows a lot drawn by
`sims-render` in a window. It opens the window with `winit` and copies
the CPU-drawn frame into it with `softbuffer`; no GPU is involved. These
two crates are dependencies of `sims-viewer` only. `sims-render` stays
free of window code, so tests draw offscreen and `cargo test` never
needs a display, and `sims-vm`, `sims-sim` and `sims-core` never see
either crate.

The frame is drawn at the original's pixel size and scaled up by whole
pixels on high-DPI screens, so pixels stay sharp. The keys follow the
original where it has one: three wall modes, quarter-turn rotations,
three zooms and two floors. The viewer reads either a house file or a
world saved by `sims boot --snapshot`, and only reads it.

The two crates are the same versions the people viewer uses (see the
entry below), so `Cargo.lock` gains no new packages. Unlike the people
viewer, `sims-viewer` is not behind a feature: it is its own crate, and
CI builds it. The Linux X11 and Wayland back ends load their system
libraries at run time, so CI needs no new system packages.

What would reopen it: the UI track needing a GPU, text or widgets that
softbuffer cannot give, at which point the window code moves to wgpu
or another toolkit, still outside `sims-render`.

## 2026-11-14: One table of IFF resources, with the prototype's rules (unverified)

`Iff::get`, `Iff::all` and `Iff::resource_order` now read one table built
when the file is parsed, as the original's IFFResFile3 keeps it. Before,
`get` and `all` scanned the raw chunks: they returned deleted chunks, and
`get` took the first of two chunks with one key even when the resource
map named only the second. `Iff::chunks` still lists every chunk in file
order, for readers that want them all.

The rules follow the frozen prototype's port of the open
(`legacy/crates/sims-sim/src/object_catalog/iff.rs`):

- A resource's key is its type, its ID and the high byte of its flags.
  `get` looks up the key whose high byte is 0, as the original's lookup
  by ID does.
- Without a resource map: live chunks in file order. A deleted chunk
  (flag 4) is skipped. A later chunk with a taken key replaces the
  earlier one in its place in the order.
- With a version-0 map: the map's entries in its order. Of two entries
  with one key, the first is kept and the second dropped unread. A map
  listing a resource map with ID 0 is ignored and the chunks are read as
  if there were none. An entry's chunk must be at its offset with its
  type, ID and flags high byte; the low flag bits are the map's to state,
  as in the prototype. (The prototype checks only the type, when it
  loads the payload; requiring the ID keeps a resource's chunk agreeing
  with the key it is found under. The game's own maps always agree.)
- A resource map is never a resource itself.
- When the map cannot be used, `resource_order` fails as before, and
  `get` and `all` use the table built as if there were no map.

**Unverified** against the original. What the game's own files hold
bounds the effect: all 4,155 deleted chunks in the install are `XXXX`
free space, none sharing a key with a live chunk; the only live chunks
sharing a type and ID are nine `BMP_` chunks that the flags' high byte
tells apart; and no map lists a map, a duplicate key, or leaves a live
chunk out. The install census and the gated digests are unchanged.
`sims-vm`'s `Program` still reads `Iff::chunks` directly (a later chunk
of one ID wins there, deleted ones included); on the game's own files
that agrees with the table.

## 2026-11-14: Bella Goth compared with the original; the mirror is still open

One pose was compared with the original game. The game ran in the
emulator harness (`tools/harness/`, its own container and guest copy),
with House05 loaded from the install and the house view at near zoom.
Bella (person 18) was walking away from the camera, and a screenshot was
taken from behind at about 27 pixels per model unit. `sims-people` then
drew person 18 from the same saved outfit at the same scale, playing
`a2o-walking-loop` at directions 3.5 to 4.5 and phases 0.05 to 0.8. The
screenshots and renders stay private, outside the repository.

What matches: the meshes (body, head, long hair, hands, shoes), the
textures (the red dress with its open back, the skin tone, the hair), the
proportions, and the hem's length. The pose falls inside the walk cycle:
it looks like phases 0.05 to 0.3.

What the comparison does not settle:

- **Mirror image: unverified.** The drawing negates file X (see
  `crate::rig`). A capture from behind has no fixed feature that is
  different on the left and right. The hem looks like one, but drawn at
  several phases its lower side flips with the stride, and the
  screenshot's phase is not known. Settling it needs a fixed asymmetric
  feature seen in the original: for example, a printed shirt or a hair
  parting, seen from the front.
- Light and shading: the renderer uses its own light, not the
  original's.
- Pixel positions: the camera was not matched to the original's pixel
  for pixel.
- The standing loop, Mortimer (person 21) and Cassandra (person 16, a
  child): the capture showed neither of them in their saved outfits.

Mortimer was also seen walking, in a work uniform (a lab coat). That
outfit is not among his saved outfits and was not found in the install,
so it was not compared. Looking for it showed that an appearance's own
name does not always match its file's name (`B629MAFitlgt_labtech` is in
`b629ma_labtech.cmx`). So appearances are now also found by their own
names across every catalog, as the old prototype did
(legacy/crates/sims-content/src/avatar.rs).

## 2026-11-14: The people viewer is a feature-gated window

`people-viewer` is a small window binary in `sims-people`. It uses
`winit` (the window and its events) and `softbuffer` (copies the
software rasterizer's image to the window). Both are optional
dependencies behind the crate's `viewer` feature, and nothing else in
the workspace enables it. So the default build, `cargo test` and CI
never compile them. They add 187 packages to `Cargo.lock`, which CI does
not download, and leave every existing package's version unchanged. The
cost is that CI does not build the viewer; run
`cargo clippy -p sims-people --features viewer --all-targets` after
changing it.

The viewer reads the wall clock to play clips in real time, so the
binary allows the workspace's determinism lint. Nothing it computes
reaches the simulation. It reads a simulation snapshot through
`sims_sim::Snapshots` and makes its own `Animator` values for the
standing and walking demonstration; it never changes simulation code.

`--png <file>` draws one frame without a window, so the viewer's
drawing can be checked headless.

## 2026-11-14: The HIT player runs real sound events into PCM (track 3, sound)

`sims_audio::player::Player` is the HIT virtual machine, with a mixer.

- It takes `SoundEvent`s: play the event named N for object O, stop
  object O's sounds, or stop everything.
- It renders mono 16-bit PCM at 22,050 Hz, the rate of nearly every
  sample in the install.
- Each event starts a thread that runs its track's HIT routine. Threads
  run at the start of every 64-frame block; notes are mixed in between.
- Random choices (hit lists, `rand`) use the player's own seeded
  generator, so a seed always replays the same sound.

What was checked:

- **Phone ring.** `Phone_ring` renders exactly the decoded
  `Phone_ring.xa`, bit for bit, then silence to the end of the block.
- **Footstep.** `footstep_soft` with `SimSpeed` at 1 renders exactly one
  of the five soft footstep samples. Eight seeds pick soft1, soft3 and
  soft4. At `SimSpeed` 2 it renders nothing, as the routine says.
- Both digests are pinned in a gated test.
- **Every event.** All 4,494 events in the install were run for 20
  blocks with stub samples, and no instruction failed. Two kinds of
  fault remain:
  - The files' own loose ends (14 faults).
  - 190 events of kinds this player does not handle yet: music modes,
    television, sequenced hit lists and others.
- Synthetic tests cover calls, branches, waits, loops, hit lists,
  resampling, clamping, questions to the simulation, and faults.

How the instructions behave comes from the HSM register names, public
notes (FreeSO, the Niotso wiki) and what the install's programs need:

- **`end` returns from a call.** The install has 2,504 calls and no
  `return`.
- **`loop` without `set_loop` goes back to the routine's start.** The
  looping generic routine relies on this.
- **`loop` waits for the next block.** Otherwise a loop that starts no
  note would spin forever.
- **`arg1` holds the object** when a track's arguments are
  `kArgsIdVolPan` (4,135 of 4,316 tracks). The routines pass `arg1` to
  `getsrcdatafield`, the one way a script asks the simulation something.

All of this is **unverified** against the original.

Left out, also **unverified**:

- Volume, pan, pitch, ducking, priority and 3D position: every note
  plays at full volume, centred, at its own speed.
- The `seqgroup_*` instructions and `test1` to `test4` do nothing.
- `setll`, `setlt` and `settl` copy like `set`.
- When modules define the same event, patch or hit list, the module
  whose name sorts first wins.
- Which value of `SimSpeed` the game uses at normal speed.

The simulation does not emit these events yet, and `sims-vm` and
`sims-sim` were not changed. What it would need:

1. **Play Sound Event (primitive 23).** It names a sound by an FWAV
   string ID. The event name is that string: looked up in the code
   owner's file, then in `Global.iff`. The simulation sends
   `SoundEvent::Play { object, name }` with the object's ID, plus the
   primitive's flags (loop, volume) once the player uses them.
2. **Stop All Sounds (primitive 48).** It sends
   `SoundEvent::StopObject` for the object.
3. **Animation clips.** Their `sound` and `footstep` events send
   `Play` too. The footstep event names are the HOT names, such as
   `footstep_soft`.
4. **`getsrcdatafield`.** It needs a read-only view of the object's data
   fields, through `sims_audio::player::Sources`. What each field
   number means is unknown. The FM routines suggest field 0 picks
   between male, female and child voices.

## 2026-11-14: The sound tables read, and an event leads to its samples (track 3, sound)

`sims-audio` now reads the sound scripts' files. A *module* is a HIT
program (bytecode) with its HSM file (symbol names) and HOT file
(tables). The install has 27 modules: two in the base game, one per
expansion pack, and one per downloaded object that has its own sounds.
The HOT tables map event names such as `Phone_ring` to tracks, tracks to
HIT entry points and patches (sample files), and list the hit lists
that scripts pick samples from. FSC files are the outdoor ambient
scores, and all four parse.

How the HIT bytecode is laid out:

- Opcode names and numbers come from the HSM files. Operand layouts come
  from public notes (the Niotso wiki, FreeSO's HIT player).
- The install confirms them for every opcode it uses. All 27 programs
  decode from their first byte to their last, and every one of the 4,363
  HOT entry points lands on an instruction.
- Branch, call and jump operands are not addresses. They are track data
  IDs that the HOT file maps to addresses; every such operand in the
  install resolves within its own module.
- `jump` has two forms: a 4-byte ID, or a 1-byte register holding the
  target. The opcode does not say which. We take the ID form when the 4
  bytes are an ID of the program. That covers 399 of the 401 jumps; the
  other two only decode as register jumps. **Unverified** how the
  original decides. Opcodes the install never uses keep the public
  layouts, also **unverified**.

What the files themselves leave dangling:

- 155 patches name sample files the install lacks. One path names a
  build machine's `c:\` drive.
- 14 events give an empty track.
- 10 track entry points use a symbol that their own module's HSM file
  does not define (`tkd_Generic`, `tkd_GenericInterrupt`). Other modules
  do define them, so the original may look across modules.
  **Unverified.**
- 4 tracks name a patch that does not exist.
- 6 hit list items are neither a number nor a range: four names and two
  dotted pairs such as `2370.2376`. These are kept apart, unread.
- The FSC scores mostly use samples in their own directory. The two big
  scores also use 56 samples from the other ambient directories, and
  six samples exist nowhere. Where the original looks for them is
  **unverified**.

The gated test pins all of these counts, so any change to the rules
shows up in it.

Traced by hand, then pinned in a test:

- **Phone ring.** `Phone_ring` plays track 3479. That track runs the
  generic routine (`note_on v1; wait_samp; end`) with the patch
  `sounddata\phone\Phone_ring.xa`.
- **Footstep.** `footstep_soft` runs a routine that sets `interrupt`,
  compares 2 with the game speed (global register 100), and branches to
  code that picks one of five soft footstep samples (hit list 217).

When two modules define the same event, the module whose name sorts
first wins (both events above are in the base game's generated module).
The original's order is **unverified**.

## 2026-11-14: Sound files decode (track 3, sound)

A new crate, `sims-audio`, reads the game's sound files and decodes them
to PCM (16-bit signed samples). The simulation does not depend on it.
Every sound file in the Complete Collection install decodes.

Terms used here. *XA* is Maxis's 4-bit ADPCM (adaptive differential PCM)
sound format. A *FAR* archive is the game's file bundle.

What was decided:

- **Three formats, no others.** The install holds 18,155 XA files, 285
  WAV files (all 16-bit PCM at 22,050 Hz) and 141 MP3 files (the radio
  stations, MPEG-1 Layer III at 44,100 Hz, stereo). No file is UTK (The
  Sims Online's codec) and no file starts with its magic number. Outside
  those extensions, only the animation `.cfp` files start with bytes that
  look like an MPEG frame header, by chance.
- **XA is decoded from public notes.** The layout follows FFmpeg's Maxis
  XA demuxer and decoder and the Niotso wiki: a 24-byte header, then
  frames of 28 samples per channel in `15 * channels` bytes. The header's
  bytes 2 and 3 hold 0, `J` or 0x12 and are not read. One stereo file
  (`sting_crystalball.xa`) ends half way through a frame and needs that
  half's samples to reach its stated size, so a partial last frame
  decodes as far as it goes. Output is cut to the size in the header.
- **MP3 uses the `symphonia` decoder** (pure Rust, MPL-2.0). Its output
  is floating point converted to 16-bit, so its digest is pinned on one
  machine only. Music is presentation; nothing in the simulation reads it.
- **Sound files are found by install-relative path.** The sound tables
  name files such as `sounddata\phone\Phone_ring.xa`; that file is the
  member `Phone\Phone_Ring.XA` of `SoundData/SoundData.far`. An
  archive's members count as files of the archive's directory, paths
  ignore case, a loose file replaces a member, and the first of two
  members with one path is kept, as in `sims_content::Index`.
  **Unverified** that the sound manager resolves paths this way.

How it was checked:

- Synthetic tests: hand-computed XA samples (each predictor, clamping,
  stereo order, a partial frame), an XA encoder written apart from the
  decoder whose output decodes to exactly what it predicted, WAV chunk
  walking, and silent MP3 frames.
- A gated test (`SIMS_INSTALL`) decodes every sound file, shadowed
  duplicates included, and pins counts per format and variant, total
  frames, and an FNV-1a digest per format. It takes about 40 seconds in a
  debug build. An ignored test decodes all eight hours of MP3 in full;
  run once in a release build (24 seconds), every MP3 file decoded and
  gave as many frames as its frame headers count.
- **Against FFmpeg.** FFmpeg's `adpcm_ea_maxis_xa` and PCM decoders
  (FFmpeg through PyAV 15.1.0, in a private virtual environment) decoded
  all 17,915 distinct XA files and 285 WAV files. After cutting to the
  size each XA header states, every file's PCM equals ours, bit for bit.
  FFmpeg does not cut: for 17,129 files it returns the rest of the last
  frame too.

Not checked against the original. The game's own XA decoder was not
traced in the executable, and the harness runs with `-nosound`.

## 2026-11-14: The lot drawing compared with the original's screen

One camera was compared: the Goth house (House05) as the game opens it,
far zoom, rotation 0, floor 1, walls cut away, with the window at 800 by
600. The original ran in the comparison harness (its own guest and
container); screenshots were taken with `xwd` and nothing was written to
the guest's memory. The screenshots stay outside the repository.

How: the render is drawn on a larger canvas, the screenshot's offset in
it is found by search, and both are reduced to the original's 16-bit
colour (5, 6 and 5 bits) before pixels are compared. Grass-green pixels
of the render are left out of the search, because the grass is not the
original's yet.

What matches:

- **The projection.** The original's camera is a pure translation of
  ours: the same offset aligns the yard, the house and the road.
- **Object sprites outdoors** (hedges, gravestones, trees, the fountain,
  the mailbox) match pixel for pixel at that offset.
- **The road and sidewalk**: 82% of pixels are exactly equal, most of
  the rest one step off.
- **Wall sprites**, once two bugs were fixed (below). With walls up,
  27% of pixels in the compared area match exactly, up from 19%. Most
  of the remaining difference is grass, lighting and the Sims.

Fixed by the comparison:

- Walls were drawn one pixel too low. The wall sprite's bottom row sits
  one pixel above the corner's projected point, not on it.
- A cut-down wall with a door or window in it showed the opening's cut
  sprite, a small outline shared by every door and window. The original
  draws the same short stub there as beside the opening, so it now takes
  the stub of plain wall style 1. Which style the original takes is
  **unverified**; the house only uses style 1 around its openings.

What does not match yet:

- **Lighting.** The original darkens floors, walls and objects indoors
  by the room's light. Everything here is drawn at full brightness, so
  most indoor pixels differ, though their shapes line up.
- **Grass.** The original's grass is a smooth, shaded texture. Ours is a
  made-up speckle, and lots whose grass value is low draw brown where
  the original is green.
- **The cap on a cut wall.** The original draws a three-row wood trim on
  top of a cut-down wall's stub. Its source is not found yet (it is not
  the wall pattern's top rows nor the 4096, 5120 and 6144 sprites of
  `walls.iff`).
- **Which walls cut away.** The original cuts walls room by room; its
  view keeps a 64 by 64 tile mask per visible room (the old prototype's
  reading in `legacy/crates/sims-sim/src/lot_presenter/room_masks.rs`).
  Right after the house opened, every ground-floor wall was still up.
  About a minute later most rooms were cut away, but the bathroom and
  the front of the round living room were not. Our rule (cut a wall
  unless it is a room's back wall) cuts them all.
- **Sims** are not drawn here; the people track draws them.

What would reopen it: a second camera (another zoom or rotation) that
disagrees, or a reading of the original's lighting or cutaway room
choice.

## 2026-11-14: Drawing the lot starts in its own crate, on the CPU

The lot is drawn by a new crate, `sims-render`. It reads the world
(`sims_sim::World`) and never changes it, and nothing in the simulation
depends on it. It draws on the CPU, so tests need no window or GPU.

Terms used here. A *sprite* is a pre-drawn picture from the game's files.
An object's sprites (SPR2) carry a *depth byte* per pixel that says how
far that pixel is from the camera. A *draw group* (DGRP) lists the sprites
that make up one view of an object. A wall's *shape* is a mask that says
which pixels of a wall exist (full wall, cut-down wall, fence); its
*pattern* is the wallpaper. *Cutaway* is the original's middle wall mode,
in which walls that would hide a room's inside are cut down.

What was decided:

- **The original's projection.** A tile's top corner is at
  `((x - y) * (8 << zoom), (x + y) * (4 << zoom))` plus the camera's
  origin, raised 58, 116 or 232 pixels per storey at zooms 1 to 3 and
  `2 << zoom` pixels per terrain step. The four rotations use the
  original's 64-tile rotation table. This is the old prototype's reading
  of the original's code (`legacy/crates/sims-sim/src/lot_presenter/`).
- **Depth is a number per pixel, smaller nearer, and a tie goes to the
  later draw.** Its scale was fitted from the game's sprites, not read
  from the original's code. For the lowest pixel of each sprite column,
  which mostly touches the floor, the depth byte falls 1.5 per screen
  pixel below the object's anchor at the near zoom, 3 at medium and 6
  at far, and is about 200 level with the anchor. So one byte is a fixed
  distance in the world. The renderer gives floors, walls and the
  anchors of objects a depth on the same scale. Whether this matches the
  original's depth tables is **unverified**.
- **Where a sprite goes.** An object's anchor is the screen point of its
  position on the floor. Each sprite of its draw group is drawn with its
  left edge and bottom at the offsets the draw group gives (checked on
  every near-zoom draw group of the Goth house's objects: the anchor sits
  at the same point of each sprite's frame). A draw group sprite with
  flag 1 is mirrored. Dynamic sprites hide when their word is zero
  (0x437f99, from the old prototype).
- **Floors** are `GameData/floors.iff` sprites `id`, `id + 256` and
  `id + 512` for the three zooms, drawn with their middle column on the
  tile's top corner. A tile a diagonal wall splits shows two floors from
  its wall cell; which half takes which is **unverified**. Grass is a
  brown-to-green mix by each tile's grass value, scattered in two
  shades. The original draws grass blades from a seeded pattern, so
  grass will not match pixel for pixel.
- **Walls** are a pattern from `GameData/walls.iff` (`id` plus 1536,
  1792 or 2048) masked by a shape (`style * 2 + cut` plus 0, 512 or
  1024), one frame per wall direction. Each wall cell owns its low-x and
  low-y walls and its diagonals; which side's wallpaper shows follows
  the old prototype's `faces`, which was compared with the original's
  wall records. A wall of style 22 or 23 takes its shape from the door
  or window standing in it (the object's OBJD byte 0xa2), as the old
  prototype did. Where a wall sprite sits on its edge was worked out
  from the shapes' outlines and is **unverified** until compared with a
  screenshot.
- **Cutaway is simplified.** A wall stays up only if the room on the
  camera's side is indoors and the other side is outdoors: the back
  walls of the house. Doors and windows in a cut wall are hidden. The
  original's cutaway (0x54059a and 0x4aa446) also depends on a mask it
  builds per view; that is not ported, so this is **unverified**.
- **Not drawn yet:** the Sims (the people track draws them), roofs,
  shadows, lighting by time of day and room, objects in other objects'
  slots raised to the slot's height, and terrain shading.
- **A small loader** (`open_house`) builds a world from a house file,
  widens its floor and wall arrays as the load does and works out the
  rooms, so a house can be drawn without a full `sims boot`.

How it was checked:

- The Goth house renders at all three zooms and four rotations, with
  every wall shape found. The renders were looked at: floors, walls,
  fences, doors, windows and furniture sit where they should, and the
  back walls stay up in cutaway. They are pinned by digest of the
  16-bit-color image (`tests/goth_house.rs`, gated on `SIMS_INSTALL`).
- Nothing has been compared with a screenshot of the original yet.

What would reopen it: a screenshot comparison that disagrees, or a
reading of the original's depth tables or cutaway mask.

## 2026-11-14: People play the simulation's clips; placement and facing

A person is drawn from the simulation's animator (`sims_sim::animator`),
read only:

- **Clips.** Every animation file (a `.bcf` that starts with eight zero
  bytes) is read once, and its clips are known by name, ignoring case. A
  clip's samples come from `<skill>.cfp`. A clip is posed at a phase from
  0 to 1: the frame is `phase × frames`, blended with the next frame,
  which wraps to the first only for a layer that repeats (end mode 1).
  This is the old prototype's sampling (`legacy/crates/sims-sim/src/pose/sampling.rs`)
  in ordinary `f32`. It is **unverified** against the original's frames.
- **Layers.** The rest pose, then the base layer's clip, then the
  foreground layer at its phase and weight. The simulation keeps only
  whether a base layer exists, not its clip or phase. Until it exposes
  them, the clip comes from the saved person record (the base descriptor,
  for the Goths `a2o-standing-loop` and `c2o-standing-loop`) and the phase
  from the animator's clock. Carry and blend layers are not drawn yet.
- **Root travel.** The walking clip's root sways forward and back by
  about 0.27 units, and returns; it does not travel. The simulation moves
  the person. So the clip's root is kept as it is, unlike the old
  prototype, which removed it.
- **Place.** Sim x and y (sixteenths of a tile) become model X and Z, at
  3 model units per tile. That figure comes from the walk step, which
  moves a person by a clip's distance times 16/3 sixteenths. Floors above
  the ground are not handled yet.
- **Facing.** Checked against the simulation, not the original: person
  259 walking for 82 ticks of `sims boot` on House05 moves along
  (sin θ, −cos θ) in sim x and y, where θ is its direction (in eighths of
  a turn) times 45 degrees. The mean difference is 0.2 degrees. Every
  clip leaves the toes pointing at model +Z, so a person is turned by
  π − θ about the vertical, as the old prototype did. The turn is computed
  with additions, multiplications and square roots only, so pinned renders
  hold on every machine.

Whether the drawing is the original's image or its mirror image is still
**unverified**, and so is which screen direction sim +x takes. Both wait
on a screenshot of the original.

## 2026-11-14: Drawing people starts in its own crate, on the CPU

The Sims are drawn by a new crate, `sims-people`. It reads simulation
state and never changes it, and nothing in the simulation depends on it.
Its math is ordinary `f32`, and it may use the platform's `sin` and `cos`
(it allows the workspace's float lint at its crate root).

Terms used here. An *appearance* is a named set of meshes bound to bones
(`b004fafit_01`, Bella Goth's body). A *texture string* is what a saved
person keeps next to each appearance, such as `BODY=b004fafitmed_fatale01`.

What was decided:

- **Files are found by their own name.** Every `.bcf`, `.cmx`, `.bmf`,
  `.skn`, `.bmp` and `.cfp` file, loose or in a FAR archive, is known by
  its name without directory, ignoring case. The base game is searched
  first, then the expansions in the order the original mounts their
  object folders, then `Downloads`. The original's own avatar registry
  is not ported, so this order is **unverified**. Every file the Goth
  family is drawn from has one source or identical copies, so the order
  does not change what they look like.
- **What a person wears comes from its saved record.** The body, the
  two hands, the head and the accessories each name an appearance and a
  texture string. An appearance is read from `<name>.cmx.bcf`, or from
  the text form `<name>.cmx`. A mesh is read from `<name>.bmf`, or from
  `<name>.skn`.
- **A texture string `KEY=texture` gives its texture to every mesh whose
  name contains `KEY`.** Other meshes keep the texture their own file
  names (Cassandra's glasses, `nerdspecs`). This is the old prototype's
  reading of the original's code (0x599e87). It explains every part of
  the three Goths.
- **The pose is computed in a mirrored frame.** Positions and rotations
  from the files have their first coordinate negated, as the old
  prototype's renderer did. Whether this gives the original's image or
  its mirror image is **unverified** until it is compared with a
  screenshot of the original.
- **Skinning follows the old prototype:** each vertex moves with its
  bone, and a blended vertex moves part of the way towards its position
  on a second bone. How close this is to the original's blend is
  **unverified**.
- **Rendering is a small software rasterizer for now.** Tests draw
  offscreen with no window and no GPU. The camera keeps sines and
  cosines, and the house view is built from square roots, so a render
  gives the same bits on every machine and can be pinned by digest.
- **The texture decoder (`.bmp`) lives in `sims-people`**, not in
  `sims-formats`, to avoid conflicts with the rendering track. It can
  move later.

How it was checked:

- The three Goths (Mortimer, Bella and Cassandra) load with the meshes
  and textures listed in `tests/goth_family.rs`, and draw in their rest
  pose. The renders were looked at: clothes, skin, hands and heads sit
  where they should, and Cassandra wears her glasses. They are pinned by
  digest, gated on `SIMS_INSTALL`.
- Nothing is compared with the original yet.

## 2026-11-14: The end of a path on a timer, the walk's strides, and K = 333

In tick 262 of the Goth house, person 21 comes to the end of a path
with object data word 9 clear. That case is now ported, with what it
needs: the walk's strides, worked out by posing the person's skeleton.
Ticks 262 to 333 then reach a few more mechanics, which are ported too.
Our state after every one of ticks 1 to 333 equals the fourth capture.
Tick 334 stops loudly. K goes from 261 to 333.

Terms used here. A *stride* is how far apart a person's feet are, along
one axis, at a footfall of the walk clip; the animator keeps two
(+0x17c and +0x178). A *footfall* is a phase of the walk clip marked by
an `anchor` event. A *pose* is where every bone of a skeleton is when a
clip is played at a phase. A *sample* is a bone's position or rotation
at one frame of a clip, stored compressed in a CFP file.

What was decided:

- **The end of a path with data word 9 clear is ported** (0x58e8be to
  0x58eb03). The phase of the walk clip picks the next footfall and the
  stride that ends there. Within that stride of the end, the animator
  sets a timer (+0x1c0) and fades the clip's weight to the share of the
  stride left (0x597a3e). A clip that may not be interrupted and has not
  finished only sets the near-end flag. When the timer runs out, the
  person stands on the end point, facing along the path's last segment.
- **A layer's fade is ported** (0x597e3c to 0x597e9b). The old test that
  expected a fading layer to fail now checks the fade instead.
- **The strides are worked out when needed, not kept.** The original
  works them out once, in the person's constructor (0x58831d), and keeps
  them. We work them out from the same walk clip and skeleton each time
  (`Interpreter::strides`). The result is the same while the clip and
  skeleton stay the same, and the snapshot does not grow. The pose code
  is new (`sims_sim::pose`). It follows the old prototype's 24-bit
  variants (`Selection::at_position` and `motion_with_control` in
  legacy/crates/sims-sim/src/pose/sampling.rs;
  `blend_quaternion_with_control` and `compose_bone_with_control` in
  legacy/crates/sims-sim/src/pose.rs). Content now reads each
  skeleton's bones and each clip's channels and raw samples.
- **The sample decoder uses the original's table.** The original turns
  each compressed sample byte into a change through a table of 253
  floats that it builds at 0x5975d4 (stored at 0x6f7188). Each entry is
  worked out in steps, each rounded to single precision. Our decoder
  used a closed formula in double precision, which differed in the last
  bits of most entries. That moved the strides by 70 to 400 units in the
  last place. `sample_deltas` now builds the table as the original does.
- **The turn on the spot takes every turn clip** (0x58f4ee to 0x58f688),
  not only the half turn. A clip already in the foreground is cleared
  first (0x592878).
- **Animate Sim names a clip from an object's own tables** (source 0,
  0x59dd6d): string tables 129 to 132, by the ages of the person and of
  the person its interaction is with.
- **Look Towards mode 0 is ported** (primitive 22, 0x45a98e, 0x58a035):
  person data words 41 to 45 hold the head's target. Modes 1 to 3 fail
  loudly.
- **Snapshot format 17** keeps the motion's timer (+0x1c0) and each
  layer's fade (+0x48 to +0x54). The pins taken under format 16 still
  hold with these left out (`format_16_bytes`).
- **The harness reads a stale spatial-order cache without failing.** The
  fourth capture stopped at update 609 with `unknown retained
  spatial-order object identity`. The registry keeps a cache of objects
  in drawing order (registry +0x4c). The only reader (0x440349) rebuilds
  it first (0x440397) when its dirty byte (+0x44) is set or its rotation
  is not the camera's. Removing an object (0x44383c) sets the dirty byte
  (0x443b65) and leaves the cache as it was. So a dirty cache can hold
  the address of an object that is gone. The harness now keeps a stale
  cache's addresses as read, with no identities, and still fails on an
  unknown address in a current cache. Its output schema is now 2.

How it was checked:

- After every one of ticks 1 to 333, our state equals the fourth
  capture's, floats bit for bit, in the same fields as before, now with
  the timer and the fade, and the `updates.jsonl` line.
- The second capture is equal at every tick it holds (to 240). The third
  is equal to 300 but for the two known lines (global 12 after ticks 1
  and 2; one local of object 18 after ticks 210 and 220).
- Our strides equal the stored ones bit for bit, for all three people
  (child 16: 0x40d893f8 and 0x40bcad30; adults 18 and 21: 0x412c5b86
  and 0x412a0544). The capture holds the same values after updates 1,
  300 and 600.
- Our table of 253 changes equals the one in a memory image of the
  original at 0x6f7188, entry for entry. In double precision, 225
  entries differ.
- `the_goth_house_first_tick_matches_the_original` runs to tick 333,
  checks the end of the paths in ticks 262 to 322 and the strides, pins
  the state, and checks that tick 334 stops at 0x5904d2.
- The load comparison still gives its three known lines.
- New synthetic tests: the frame choice, sampling, blending and
  composing of a pose; the fade; the start of a stop on a timer; the
  table's digest; Animate Sim's own tables; and Look Towards mode 0.
- The harness change was checked only on made-up memory. No capture
  has been run with it.

What did not match, and why:

- Nothing new. At the stop in tick 334, person 16's person word 27 is
  12 in ours and 13 in the capture after the tick. The steps after the
  stop presumably write it, as at the stop in tick 262.
- The first capture still differs from tick 233 on, as before.

Unverified:

- To pose the skeleton for the strides, the layer's time since its last
  pose is taken as 0 (so the next frame is always blended in) and the
  clip as looping. Neither is read from the original. The strides this
  gives equal the stored ones.
- That a removed object was what stopped the fourth capture at update
  609. The code says a dirty cache can hold one, and the dirty byte was
  set after every update the capture saved; the memory at update 609
  was not saved.

## 2026-11-14: A person walks through a door, and K = 261

In tick 233 of the Goth house, person 21 routes to an object in another
room. Routing between rooms through doors is now ported, with what the
walk reaches next: slots searched by distance, the end of a path, and a
room noting who is in it. Our state after every one of ticks 1 to 261
equals a new fourth capture of the original, which holds the full state
after every update to 608. Tick 262 stops loudly. K goes from 232 to
261. Why the first capture chose a tick later than the others is not
known, and cannot be told from what can be recorded: this is a stop
condition for modeling it.

Terms used here. A *portal* is a door seen as a way from one room to
another. A *capture* is a run of the original under the harness that
saved its state. A *frame* is one drawing of the house view, as far as
it writes the simulation's state.

What was decided:

- **Routing between rooms is ported** (`routing/portals.rs`). People and
  doors take numbers from one pool (object data word 6, 0x46381a; reset
  at 0x4638b9). Each door holds a vector of costs (object +0x1ac), one
  column per number. A door's column holds its distance to every other
  door through rooms and doors (0x46390f). A person's column holds the
  cost of reaching the target through each door of its room (0x463c83:
  10001 for no way, 10000 for a door the route may not use, 1101 for a
  door that failed, 1001 to 1100 for one tried before, the distance
  otherwise). Step 2 of the route (0x45afd0) takes the cheapest door,
  notes the try (0x463e43) and pushes the door's tree on the person's
  thread. A second try, or a door tree that returns false, marks the
  door failed (0x463e09).
- **Slots are searched by distance** (0x483598). Each place is
  classified (0x481439): off the lot, another room, closed off by walls
  or an object (0x4359ea), uneven ground, or free. A place is turned by
  the slot's direction (0x483d4b). The last step of a route to a slot
  takes the slot's facing (0x45c516 to 0x45c5fb).
- **The planner plans indoors and upstairs** (a free rectangle costs 0
  there, 0x4824d0 to 0x48253e), and without the width limit in a small
  or crowded room. The ratio of objects to half tiles is rounded to
  single precision before it is compared with 0.2 in double precision
  (0x468103, 0x46810a), so exactly a fifth counts as crowded.
- **The end of a path is ported for a person with object data word 9
  set** (0x58e740). It walks on until it is near the end point, then
  stands. Standing with the route's end noted clears the walking flag
  (0x58f117). The route then ends (route_result 0x589d47, end_route
  0x45b6ea; a route under way does not stop, 0x589e2d). The person is
  put on its tile and turns along the last segment.
- **Rooms note who is in them** (0x468802). A person whose room changes
  tells both rooms. A dark room with no lamp then shows the light of a
  lit room (0x4688bf), so it waits to be counted again. A person's visit
  in the load notes its room too (0x45e43d, called at 0x459c22).
- **Each frame counts the rooms that wait** before it draws (0x4a8895,
  called at 0x4a8b03). So the frame schedule changes the rooms' state.
- **The default frames are `0,1,2,3,33+30`**: before tick 1, after
  ticks 1, 2 and 3, after tick 33, and after every 30th tick from there.
  The three captures that watched the draw guard all show this. The
  earlier default, a frame after every tick from 33 (`0,1,2,3,33+`),
  counted the rooms sooner. The `N+S` form is new in `sims boot
  --frames`.
- **Snapshot format 16** keeps each room's mark (Room +0x73) and the
  animator's last step (+0x294, +0x298), its near-end flag (+0x288) and
  its finishing flag (+0x28c). The pins taken under format 15 still
  hold with these left out (`format_15_bytes`). The states after ticks
  5, 13, 33 and 232 were pinned again, because the rooms now hold other
  values from the frame after tick 2. Everything the captures hold is
  the same before and after.
- **A third and a fourth capture were made.** The third holds 300
  updates (full state after 1 to 100 and every tenth, 26 MB). The fourth
  holds the full state after every update to 608 (130 MB). Both watched
  the draw guard. The harness now saves each door's portal costs as
  bit patterns (`portal_bits`). The fourth stopped at update 609: the
  harness found an object in a retained list that the registry did not
  hold (`ValueError` from `qemu_retained_owners.py`), and it refuses to
  guess. The captures stay private under the old checkout's `cache/`.

How it was checked:

- After every one of ticks 1 to 261, our state equals the fourth
  capture's, floats bit for bit: every field listed in
  `the_goth_house_first_tick_matches_the_original`, the people's
  animators (with the new fields), the route contexts and paths, all 71
  doors' portal costs, and the `updates.jsonl` line (tick count, both
  random streams, the C runtime stream, the scheduler cursor).
- The second capture is equal after every tick it holds (to 240). The
  third is equal but for the two lines below.
- `the_goth_house_first_tick_matches_the_original` runs to tick 261,
  pins the state there, and checks that tick 262 stops at 0x58e8be.
- The load comparison still gives its three known lines. Ticks 1 to 232
  still match.
- The new mechanics have synthetic tests: the pool and columns, the
  distance between doors, choosing and trying doors, closed directions,
  the turn, the slot order of refusals, the planner's two settings, the
  end of a path, and rooms noting people in a tick, a visit and a frame.

What did not match, and why:

- Global 12 after ticks 1 and 2 is 17 in the third capture (and the
  first), and 0 in ours, the second and the fourth. It is the
  interface's wait check, which we do not run.
- One local of object 18's seventh frame after ticks 210 and 220 is
  16104 in the third capture, and 0 in ours and in the fourth.
- At the stop in tick 262, person 21's person word 27 is 40 in ours and
  41 in the capture after the tick. The steps after the stop presumably
  write it.
- The first capture does not choose in tick 233 (below).

**The first capture's tick 234: not modeled, likely the harness's pause.** Its state after
tick 230 equals ours in all compared fields, and its streams are equal
to 232. After tick 240, person 21 runs tree 0x208a node 10 with no
running action: it chose a tick later. Ruled out:

- The frames. Fifteen schedules in our model all choose in tick 233.
- The dialogs. Notices can close without a press (one run showed it),
  so the presses are not the input.
- The cursor. It differs between runs, but runs with different cursors
  both chose in tick 233.
- The state. The original's code, run in an emulator from the first
  capture's own memory at update 47, chooses in tick 233.

What differs: only the first capture was stopped and taken over at
update 45 (its events: `attach resume=True updates=45`,
`took_over_at_return`), so the game stood still for a while. Object
+0x154 and +0x158 look like timestamps in milliseconds, and they differ
between runs. Find Best Interaction checks interface state before it
chooses (0x45ddc0 to 0x45dde2): the selected person (0x4e97dc),
[0x6e48bc]+0x10c, and byte 0x6f2a54 (written at 0x4ef21f). Any of these
may depend on timing. The captures do not hold what would tell, and
timing cannot be recorded as an input with this harness. So it is not
modeled; we follow the other three captures, which agree with each
other and with ours. This is not a roadmap stop condition: the original
is observable, and three runs that were not paused for long behave the
same. The likely cause is the long pause at update 45, which a normal
run does not have; that is **unverified**. An experiment that pauses a
run on purpose would test it. If the original's choices turn out to
depend on wall-clock time in normal play too, a fixed time input will
be needed, as for the frames.

Found wrong along the way:

- `draw_frame` did not count the waiting rooms, though the doc in
  `rooms.rs` said frames did. Without the count, motive 13 of a person
  differed from the fourth capture after tick 260.
- The planner's doc said "at most a fifth"; the single-precision
  rounding makes exactly a fifth too many.
- The slot's three proximity words were named minimum, optimal and
  maximum. The search (0x483598) reads them as the least, greatest and
  best distance, in that order.
- The walking flag was not cleared when a person stood at the end of a
  route, and a visit did not note its room.
- Route context word +0x68 had no meaning in our docs. It holds the
  door the route goes through.
- A doc comment about cells in `placement/find.rs` sat on the wrong
  function.
- The last entry's claim that a frame after every tick from 33 gives
  the same state after tick 232 no longer holds for the rooms (it still
  holds for what the captures hold). Its default `0,1,2,3,33+` is
  replaced.

**Unverified:** that route context word +0x74, which the original leaves
as the heap held it, is never read; which of the candidates above made
the first capture choose later; that nothing outside the compared fields
depends on the frames within updates 1 and 2; that the draw's other
writes do not matter; that the steps after the 262 stop write person
word 27.

## 2026-11-14: A person chooses an interaction by itself, and K = 232

In tick 233 of the Goth house, person 21 chooses what to do by itself.
Find Best Interaction (primitive 3) and Gosub Found Action (primitive 30)
are now ported, and so is the first part of Go To Relative Position
(primitive 27). The tick then stops loudly, because the person's route
goes through a door, and routing between rooms is not ported. A second
capture of the original holds the full state after tick 232, and ours
equals it. K goes from 230 to 232.

What was decided:

- **Find Best Interaction is ported** (0x45dd39, `autonomy.rs`). It
  scores each allowed interaction table entry against the person's
  motives, with the person's score curves, in single precision. It sorts
  the candidates with the C runtime's `qsort` (0x6356ba), because the
  order of equal candidates decides the choice. Then it draws once from
  the lot's random stream, unless the best entry or the person skips the
  draw. Its constants come from the global file's tuning table 2, with
  the original's defaults (`Program::autonomy_constants`). The check
  trees it runs can read and change their entry's advertisements (memory
  scopes 12, 28 and 29). What the house does not reach fails loudly:
  free will off for a pet or a visitor, a lot that is not zoned or is a
  community lot, joining another person's interaction, and a visitor's
  entry that asks the relationship.
- **Three primitives that the check trees use are ported**: Alter Budget
  (25), only its test; Test Sim Interacting With (37); and Add/Change
  Action String (50), only inside a check tree, where it does nothing.
- **Gosub Found Action is ported** (0x45a859, `found_action.rs`). It
  pushes the chosen entry's action tree on the person's thread, and the
  person starts using the object. The action becomes the person's
  running action (0x45e6cd, `Machine::set_action`). A new action with a
  target and no number takes the next number from a counter in the
  running game (0x6e4904). No save holds that counter, so it is kept in
  `World::action_serial` and starts at 0. Snapshot format 15.
- **Go To Relative Position builds its slot** (0x45c634,
  `relative_slot`) and then takes the same route steps as Go To Routing
  Slot. A target in another room fails loudly at 0x45b140: the route
  then goes through a door that 0x45c7c2 and 0x46390f choose, and that
  is not ported. A smaller exact K is better than a larger one that is
  not exact.
- **A second capture was made**, to see tick 233. It holds 240 lot
  updates, the full state after updates 1 to 5, 30 to 35 and 225 to 240,
  and a memory image at the entry of update 233. It also watched the
  house view's draw guard, the candidate list, the choice and the action
  counter. The view drew a frame before update 1 and after updates 1, 2,
  3, 33, 63 and every 30th to 213. Our run uses those frames when it is
  compared with this capture. With a frame after every tick from 33
  instead, our state after tick 232 is the same.

How it was checked:

- Our state after each of ticks 1 to 5, 30 to 35 and 225 to 232 equals
  the second capture's, field by field, floats bit for bit, in the same
  fields as for the first capture. The `updates.jsonl` line is equal
  after every tick from 1 to 232.
- The first capture still matches as before: every tick to 100, every
  tenth to 230, and the `updates.jsonl` line to 232.
- In tick 233 the watchpoints saw six candidates, the choice of menu 6
  (0x45e28b), and the action counter going from 0 to 1 (0x41f15e). Ours
  does the same. Find Best Interaction was also compared step by step
  with the original's code, run on a memory image at the entry of
  update 233.
- At our stop in tick 233, these equal the capture's state after the
  tick: the lot's random stream, the action counter, person 21's running
  and last actions, its frames, its use of object 132, object 132's user
  count, and every object that ran before person 21.
- `the_goth_house_first_tick_matches_the_original` runs to tick 232,
  pins the state, and checks where tick 233 stops and what it holds
  there. The pins taken under format 14 still hold with the new counter
  left out (`format_14_bytes`).
- The new primitives have synthetic tests.

What did not match, and why:

- At our stop, person 21's data differs from the capture's. The capture
  went on past the stop: it numbered the person for the doors (object
  data word 6), pushed two frames of the door's tree, and put the door
  in use. Object data word 9 and person data words 27 and 37 also
  differ there. We do not run those steps.
- The first capture does not choose in tick 233. Its one draw of ticks
  233 and 234 comes in tick 234. Its state after tick 232 was equal to
  ours as far as we compared. The second capture, and the original's
  code run on the first capture's state from update 47, choose in tick
  233. So the original does not always do the same here. We follow the
  second capture.
- Global 12 after tick 1 is 17 in the first capture and 0 in the second.
  Ours is 0.

Found wrong along the way: an entry with no check tree failed loudly in
our Find Best Interaction; it is now allowed, as in the original. The
house did not reach that case. The comment on Alter Budget named the
wrong scopes; the code was right (0x44eb02: a literal, an argument, a
local).

**Unverified:** why the first capture chose a tick later; the end of an
action and of a use (no use ends in the ticks that run); whether the
steps the capture took past our stop wrote object data word 9 and person
data words 27 and 37; Alter Budget's spending, and Add/Change Action
String outside a check tree, which fail loudly.

## 2026-11-14: The house view's frames are modeled, a person walks, and K = 230

Our state now equals the 1000-tick capture of the Goth house after each
of ticks 1 to 100 and after every tenth tick from 110 to 230, in
everything compared, but for global 12 after tick 1 (already known). The
capture has no full state after ticks 231 and 232, and tick 233 stops at
Find Best Interaction (primitive 3), which is not ported. K goes from 33
to 230.

Some terms. A *frame* is one drawing of the game's window; the *house
view* is the part that draws the lot. A *write watchpoint* is a hardware
breakpoint that stops the game when it writes given bytes; it gives the
instruction that wrote (its address, EIP) and, through the saved frame
pointers, the return addresses of the calls that led there (the *call
chain*). The *animator* and its *state* are as in the entry below.

What was decided:

- **The first frame that draws a person writes its animator, and that
  is modeled** (`crates/sims-vm/src/frame.rs`,
  `Interpreter::draw_frame`). After tick 34 the capture had person
  259's direction and destination direction at 0. A run of the original
  with a write watchpoint on the direction (`capture.py --watch`)
  stopped at 0x584895, outside the lot update, between updates 33 and
  34. Call chain: 0x45e36b (the person's draw), 0x4ab67d, 0x4a945d,
  0x4a8deb (the house view's draw), 0x4d1bbb, 0x4d0dea
  (`DDDSimsView::Draw()`), 0x5dcffe (the main loop). It is the
  animator's draw (0x590c05). Its byte +0xdb is set when the animator
  is made and cleared by the first draw, which also unsets the position
  (0x589361) and places the animator on its person's tile, facing the
  person's direction word (0x592132). Drawing is presentation, but this
  writes state the next tick reads, so it is modeled. The rest of the
  draw (the pose, bounds, screen rectangle, the skin's censor update) is
  not; that it writes nothing the simulation reads is **unverified**.
  Where the draw would need more than this (a skin waiting for its
  update, a head turn, a body that follows person word 8, a person in
  the house view's second list), it fails loudly. Byte +0xdb is kept as
  `Animator::undrawn`; snapshot format 14.
- **When frames are drawn is an input.** A second run watched the house
  view's draw guard (written at 0x4d1b60 and 0x4d1be7). The view drew
  before the first update, after updates 1, 2 and 3, and after update
  33, and not after updates 4 to 32 or 34 to 45. People 16, 18 and 21
  were drawn by the first frame (their +0xd8 to +0xdb were already 0
  before update 1). That timing comes from the main loop and the window
  system, not the simulation, so `sims boot --frames` takes it, and the
  default is `0,1,2,3,33+` (0: before tick 1; 33+: after every tick from
  33 on). Checked: with that default, and with `0,33+` or
  `0,1,2,3,33`, the state after tick 232 is the same and every compared
  tick is equal; with only `0`, 82 lines differ from tick 34 on. Why the
  original drew nothing after updates 4 to 32 is **unverified**; it
  depends on how the harness paces the game.
- **Registry byte 0x12 is set at the end of the object read**
  (`World::finish_object_read`). A watchpoint on it stopped at 0x43fc8d
  (in 0x43fc02, after load steps 8 and 9), going from 0 to 1. It stopped
  again at 0x442521, in update 36, going to 0: the end of a full lot
  update clears it when the tick count is a multiple of 40 (0x4424cb),
  now ported too. Both are simulation. The load's state, and
  the state after tick 1, now match the capture on this byte; before,
  ours set it in tick 2.
- **The walk is ported** for what the house reaches (0x58eb29 to
  0x58f089, `crates/sims-vm/src/animator.rs`). Facing the point ahead,
  the animator starts the walk clip repeating (layer end mode 1,
  0x597dca), keeps the old clip's phase if that clip moves the person
  (clip +0x34, `Clip::moving`), moves each tick by the clip's distance
  times 16/3 over its duration at its speed in ticks of 42 milliseconds,
  moves the step onto the path where that is nearer (0x48313e,
  0x45ed5f), puts the person on the step's tile with the place test,
  and notes its room (0x45e43d, moved out of the person's tick as
  `note_room`). Byte +0x19c, whether it walks, is kept
  (`Animator::walking`). A walk that would stop short (0x58ed52) or is
  blocked (0x58f682) fails loudly.
- **Timed events are ported** (0x5912fd, `clip_events.rs`): `xevt`,
  `interruptible`, `lefthand`, `righthand` and `censor`. Sounds are not
  modeled. `anchor` is not kept: it feeds only code behind byte
  0x6f6d70, which no code writes and which is 0 in a memory capture of
  the running game. That nothing else reads a layer's anchor fields is
  **unverified**. `dress` and `undress` fail. Animate Sim's clip 0
  stops the foreground clip (0x5897aa).

How it was checked. The capture's full state after each of ticks 1 to
100 and every tenth tick to 230, and its `updates.jsonl` line after
every tick to 232, were compared with ours field by field, floats bit
for bit, by a script outside the repository: the fields listed for tick
1 in `the_goth_house_first_tick_matches_the_original`
(`crates/sims-vm/tests/resize.rs`), the four people's animators with the
walking flag, and each person's number of route contexts. That test runs
to tick 232, pins the state, and checks that tick 233 fails. The pins
taken before this change still hold once the new fields and byte 0x12
are put back as they were (`format_13_bytes`, `with_read_flag_clear`).
The walk, the path snap, the repeat mode and clip 0 have synthetic tests.

What did not match: global 12 after tick 1, as before.

**Unverified:** the state after the ticks the capture skips (101 to 109
and so on); the draw's other writes; the frame schedule outside the
45 updates the second run watched.

## 2026-11-14: A person routes to a slot, and ticks 14 to 33 match (K = 33)

In tick 14 of the Goth house, person 259 goes to a routing slot of
object 51 (Go To Routing Slot, primitive 45). That route is now ported:
the route context, the places to stand, the planner and the animator's
start of the route, its following of the path and its turn on the spot.
The end of a clip in Animate Sim is ported too (tick 24). Our state
after each of ticks 1 to 33 equals the capture. K goes from 13 to 33.

Some terms. A *route context* is the record a routing primitive makes
for a person and pushes on the person's list (person +0xaf0); it holds
the slot, the places the person could stand (*candidates*) and the one
chosen. The *path* is the list of points the planner gives (person
+0xae0). The *animator* is the person's object that plays clips and
moves the person along the path; its *state* says what it is doing (3
standing, 4 turning on the spot, 9 following the path). A *clip* is one
animation; the *foreground slot* holds the clip a script started.

What was decided:

- **Go To Routing Slot is ported for the route the house takes**
  (`crates/sims-vm/src/primitives/go_to_slot.rs`). Its steps run from
  the frame's primitive state, as in 0x45ad63: make the context
  (0x48163e), find the candidates (0x48235a), plan (0x45c38e), start
  the route (0x589c85), then wait while the route is under way. Every
  other step and result fails loudly. The slot's index is the operand
  word at byte 0. The word at byte 2 says where the index comes from;
  only 1, a literal, is ported.
- **The routing state is part of the lot** (`sims_sim::route::Routing`,
  `World::routing`): each person's contexts and path, the slot flags
  that differ from the type's, and the person words the original wrote
  from memory it never set.
- **Person word 37 is held as unknown.** The planner copies a
  candidate's word +0xe into it (0x45c418), and the slot search never
  writes that word. The original's value is whatever its memory held
  (3287 in the capture). We store 0 and mark the word unknown; a script
  that reads it fails loudly, and a write makes it known.
- **The planner is ported** (`routing/planner.rs`, `regions.rs`,
  `rooms.rs`, `search.rs`), from the original's code and the old
  prototype's search (legacy/crates/sims-sim/src/lot_route/search.rs,
  legacy/crates/sims-sim/src/region_path.rs,
  legacy/crates/sims-sim/src/regions.rs). An object's obstacle keeps its
  footprint's words in their own order. An obstacle that costs 8 or
  more, or is a person, grows by 3. Only an outdoor room on the ground
  of at least 100 tiles is ported.
- **The animator follows a path and turns on the spot**
  (`crates/sims-vm/src/animator.rs`): state 3 (0x58ddbf), the start of a
  turn of more than 3 eighths the negative way (0x58df01), state 4
  (0x58e045) and state 9 (0x58e4e0, 0x58eb29). A new animator and a reset
  one have segment -1 (0x587d2c, 0x589220). Angles use the FPU's
  `FPATAN`, which `sims-core` gives as `Ext::atan2` through libm.
- **A clip's end is ported** (`primitives/animate.rs`, 0x589c25): the
  animator stops playing, takes its state from the posture (0x592904:
  0 gives 3, 1 gives 1, 2 gives 2), and brings its base layer up to
  date (0x590430). The pickup and person word 19 are cleared, and the
  true branch is taken. Starting the next clip clears the finished one
  from the foreground slot at once (0x586b63). The original fades it out
  instead when byte 0x6f6bd0 is set; that byte is 0 in a dump of the
  running game, and the fade is not ported.
- **Snapshot format 13** adds the route fields of each animator and the
  routing state. Every older pin in `tests/resize.rs` and
  `sims-sim/tests/world.rs` still holds once the new fields are taken
  out (`format_12_bytes`), so the pinned states are the same.
- **The harness can dump the game's memory** (`tools/harness/qmp.py`,
  `capture.py --dump`). QEMU's `memsave` reads the stopped guest; nothing
  writes it. The dumps feed an emulator of the original's code that is
  outside the repository.

How it was checked:

- Our state after each of ticks 1 to 33 was compared with the capture's
  after the same lot update, field by field, floats bit for bit. The
  fields are those of the previous entry, plus the animators' route
  fields and each person's number of route contexts. The list is in the
  doc comment of `the_goth_house_first_tick_matches_the_original`.
- The capture does not hold the context or the path. A new run of the
  harness dumped the original's memory at the entry of updates 14, 15,
  47 and 580. The original's update was run in an emulator on the dump
  of update 14. Its context, the planner's obstacles, room rectangles
  and goals, both searches, and the 11-point path equal ours.
- Ticks 1 to 13 still match, and the load comparison with its trace
  gives the same 3 lines as before.

What did not match, and why:

- After tick 1, global 12 and registry byte 0x12. These are the known
  differences of the previous entries.
- After tick 34, person 259's animator. The capture has 0 as its
  direction and its destination's direction; ours has the turn's (0.127
  and 0.210). The emulated update writes the same direction as ours, at
  0x58488f, so the 0 is written outside the lot update. The dump of
  update 47 also has 0.
- From tick 36, registry byte 0x12 (ours 1, the capture 0). Its cause
  was not looked into.
- Tick 46 does not finish. Person 18 starts a clip with timed events
  (0x5912fd), which is not ported.

What is **unverified**:

- What writes 0 into person 259's animator direction after tick 34. A
  guess is the interface: an animator's position can be unset (virtual
  function 0x20, 0x589361) and placed again from the person (virtual
  function 0x60, 0x592132). Nothing ported runs the interface.
- That `Ext::atan2` gives the FPU's value. libm is correct to within one
  unit in a double's last place; the game rounds to 24 bits, so only an
  angle within about 2^-52 of a rounding midpoint could differ.
- That the sound events of a footstep change nothing the simulation
  keeps. They are not modeled.
- The branches of the routing the house does not reach. They fail
  loudly. The synthetic tests check the obstacle rule, the following of
  a path and the turn against our reading of the code.

What would reopen this: a difference found in ticks 14 to 33, or the
writer of the animator's direction after tick 34.

## 2026-11-14: A script can make a person, and ticks 6 to 13 match (K = 13)

In tick 6 of the Goth house, object 382 makes a new person (Create New
Object Instance mode 8). That is now ported, and so is what the next
seven ticks reach. Our state after each of ticks 1 to 13 equals the
capture of the original. K, the number of ticks after each of which our
state equals the capture's, goes from 5 to 13. Tick 14 stops loudly at
routing.

Some terms. A *capture* is the private record the harness took of the
original running the Goth house: its full state after each of lot
updates 1 to 100, and one line per lot update in `updates.jsonl` with
the tick count, the three random streams and the scheduler cursor. An
*object use* is an entry in a person's list of the objects it is using
(person +0xb00). The *usage counts* are six pairs of numbers in the
house file's model (HOUS 1): for each of the last five days and today,
how often a household member was counted, and how often while routing.

What was decided:

- **The person factory is ported** (`crates/sims-vm/src/person_factory.rs`,
  `Interpreter::create_person`), following the person path of 0x440b2d
  from 0x440d63: a guard against pets, the lowest unused ID, the
  storage, the skeleton (which draws from the C runtime's stream), the
  neighborhood record, default traits if the interests are low, the
  construction reset (0x4598e8: Init twice, the first update once, the
  animator reset), and the price and ready flag. Ported from the old
  prototype's `PostConstruction`, `ConstructionReset`, `InitialReset`,
  `default_traits` and `attach_person_neighbor`
  (legacy/crates/sims-vm/src/person_factory.rs).
- **The pet guard fails loudly.** A person type whose skeleton is `dog`
  or `kat` is made only if a field of the running game (+0x66 of the
  object at 0x6e48bc) has bit 0x20 or 0x80 (0x440d8b to 0x440dab). That
  field is not modeled.
- **Mode 8 makes a person** (`primitives/create.rs`): the factory, then
  the record's interests and personality (0x425930), then, with flag
  0x20, the stack object's facing whole (0x44e684). A new person whose
  person word 84 or 68 has bit 0 set, or word 74 bit 0 or 1, goes
  through 0x45810c, which fails loudly.
- **The place test handles a person** (`placement/find.rs`): a person
  skips the wall rules (0x432ac5); one with person word 68 bit 0 collides
  with nothing; another object is in its way as 0x433975 says (when not
  solid, with data word 8 flag 2; when solid, without flag 0x10). Person
  word 84 bit 0 fails loudly (0x43383f). What the interface does when a
  person comes onto the lot (0x44752e) fails loudly in three cases (the
  selected Sim, person word 77 above 0, person word 81 of 10 or more)
  and otherwise does nothing.
- **The house's usage counts are state** (`World::usage`,
  `HouseUsage`). They are read from the house file's model with the
  simulator state (0x416568; `World::read_house_model`). Every tenth
  tick, each awake household member adds one to today's total, and one
  to today's routing count if it is in the middle of a route (0x45ce14
  to 0x45ce5c, 0x4165e3). The roadmap said before that the person asks
  the lot something there; that was wrong.
- **Object uses are counted** (`crates/sims-vm/src/object_uses.rs`).
  When a person runs a tree of another object with Run Tree By Name in
  mode 2, the person's virtual function 0x34 (0x45e59b) adds an entry
  and the object's data word 62 goes up. After each frame a person's
  thread pops, its virtual function 8 (0x45e5fd) removes the last entry
  if its frame is gone. When the list empties, the running action ends
  (0x45e6cd). The result of a return is written after that (0x474da0),
  so the ended action's flag 0x20 takes the result before the return.
  Ported from the prototype's `push_target_tree_args`,
  `pop_person_reset_frame` and `clear_person_action`.
- **The first clip reads the appearance again.** A new animator has its
  flag +0xd8 set (`Animator::profile_stale`); looking up the first clip
  reads the person's appearance and clears it (0x5893b3).
- **Snapshot format 12** adds that flag at the end of each animator and
  the usage counts at the end of the encoding. Every older pin in
  `tests/resize.rs` and `sims-sim/tests/world.rs` still holds once the
  new fields are taken out (`format_11_bytes`, `without_usage`), so the
  pinned states are the same.

How it was checked: our state after each of ticks 1 to 13 was compared
with the capture's after the same lot update, field by field, floats
bit for bit. The fields are listed in the doc comment of
`the_goth_house_first_tick_matches_the_original`: the `updates.jsonl`
line; the tick count, streams, cursor, funds, globals, removal queue,
registration order and set of objects; the 463 neighborhood records,
the inventories and registry byte 0x12; every object's GUID, data
words, temps, attributes, scheduler word, last result, frames,
relationships, slots, position and marks; every person's data, motives,
actions, queue and objects in use; and the people's animators. 617
lines are equal. Ticks 1 to 5 were also compared again with the first
capture, and the load with its trace; both give the same differences as
before.

What did not match, and why:

- After tick 1, global 12 (17 in the original, 0 here) and registry byte
  0x12 (set in the original from its load, here from tick 2). These are
  the known differences of the previous entry.
- Tick 14 does not finish. Person 259 reaches Go To Routing Slot
  (primitive 45, 0x45abce), the start of routing, which is not ported.
  Where it stops, person 259's frames and objects in use and the 258
  objects before it in the tick equal the capture's after tick 14.

What is **unverified**:

- The usage counts. The capture does not hold them, and nothing ported
  reads them.
- The end of an object use and of a running action. No use ends in the
  ticks that run.
- One local of person 259's third frame at the tick 14 stop. The
  original left half of a pointer there (2623); we hold it as unknown,
  and it is not compared.
- The parts of the person factory, mode 8 and the place test that the
  Goth house does not reach. The synthetic tests check them against our
  reading of the code.

What would reopen this: a difference found after tick 13, or a capture
that holds the usage counts.

## 2026-11-14: The comparison harness runs again, and a capture holds 1000 ticks

The original runs again in the emulator and can be stopped and read at
every lot update. `tools/harness/` holds the scripts. A new private
capture of the Goth house holds the state at the entry of tick 1, after
each of the first 100 lot updates, and after every tenth up to 1000.

What was decided:

- **The harness is the old prototype's, cut down.** Debian bookworm
  with `qemu-system-i386` in Docker. An i386 guest with full emulation
  (TCG) runs `Sims.exe -w -nosound` under Wine. Hardware breakpoints
  stop the game at the entry of the lot update (0x46d345) and at the
  end of its body (0x46d51d). State is read through QEMU's GDB stub.
  Nothing writes guest memory or registers. The only inputs are the
  button presses the README lists.
- **The guest reads the install's own UserData.** The guest's UserData
  folders are replaced with copies of the install's, and their hashes
  are checked before the game starts. With that, the C runtime stream
  and all 463 neighborhood records match ours at the end of the load.
  The first capture's 52-draw offset and its five differing records
  came from that guest's own files, not from the code.
- **Dialogs are answered as in the first capture.** OK to the five
  notices in tick 1 (read with status 2), and No to the Help System in
  tick 2 (status 3). The game closed two later dialogs itself, in
  updates 901 and 902.
- **Captures stay private.** They live in the old checkout's ignored
  `cache/`. The guest disk, kernel, initramfs and ISO are inputs that
  the repository does not hold.

How it was checked: our state at the entry of tick 1 and after each of
ticks 1 to 5 was compared with the new capture field by field. 248
lines are equal and 3 differ:

- Registry byte 0x12 is already 1 when the original's load ends. Ours
  sets it in tick 2. This is two of the lines (load and tick 1).
- After tick 1, global 12 is 17 in the original and 0 in ours. Global
  12 is the interface's wait check, which we do not run. The first
  capture was read at the entry of the next lot update, not at the end
  of tick 1, and had 0 there. **Unverified:** that the open dialog sets
  it and something outside the lot update clears it.

Known gaps in the capture: the animator pointers were not recorded for
updates 1 to 45. For ticks 1 to 5 the comparison uses the pointers
recorded at update 46, on the same pages.

What would reopen this: a capture whose load does not match ours from
the install's files, or a need to observe something the GDB stub
cannot reach.

## 2026-11-14: Find Location For is ported, and ticks 2 to 5 match (K = 5)

Find Location For (primitive 16) is ported for the modes the Goth house
reaches. With it, tick 2 runs to its end, and ticks 2 to 5 run. Our
state after each of ticks 1 to 5 equals a private capture of the
original after the same tick, in everything compared, the C runtime's
random stream aside. K is now 5. Tick 6 is beyond the capture and stops
loudly.

Some terms. The *place test* is the check whether an object may stand
at a position (virtual function 0x54, 0x433311). The *search* is the
lot's walk over tiles in a square spiral out from a start tile, looking
for one that passes its tests and the place test (0x4347f0). The *C
runtime stream* is the random number generator of the C library
(`rand`), apart from the game's own two streams.

What was decided:

- **Find Location For** (0x44b3c8, `primitives/find_location.rs`).
  Mode 0, and modes 6 and up, which the original treats the same: the
  search starts from tile (1, 1) on level 1, or, with flag 1, from the
  object a local names, and looks first in the start tile's
  room, then anywhere. Flag 2 allows a tile that already holds an
  object; flag 4 refuses tiles with ground flag 0x20. The tile found is
  tested once more, the object is put there with the lot told (0x433340)
  and the true branch is taken. Modes 1 to 5 fail loudly; mode 5 is the
  one that draws random numbers. Native errors 0x15, 0x33 and 0x17 are
  kept.
- **The search and the place test** (`placement/find.rs`), written from
  the original's code: the spiral (0x434ba9), the search's own tile tests
  (a diagonal wall, an empty grid cell, the same room, a pool tile, the
  ground flag), and the place test through 0x4329c1: the border and
  level, the walls the object needs or cannot stand against (0x476f5a,
  only for an object with no wall rules), water, pool, slope, floor,
  tile flag 4, the objects on the tile and the eight around it (0x4337f6)
  and the walls the footprint would cross (0x476dbc). A person, a tile of
  a group, a container, the upper level's floor test (0x4330b7), wall
  rules and two objects that are both not solid fail loudly. The old
  prototype's ports (legacy/crates/sims-sim/src/placement.rs `Spiral`,
  `search_tile_allowed`; legacy/crates/sims-sim/src/lot_acceptance.rs)
  were read alongside; they were checked only against the original's
  code run alone, which is weaker evidence than the capture.
- **Clip names ignore case when nothing matches exactly.** The script of
  people 18 and 21 names the clip "...-Rstand"; the animation library
  has only "...-rstand". The library's lookup (0x5a27f2) compares bytes
  exactly, but the capture shows the original playing "...-rstand" in
  tick 3, with the duration and frame count of that clip. Where the
  original ignores case was not found in its code. `Animations::clip`
  takes an exact match first, then the single name that differs only in
  case, and fails when there are two.
- **Sample names lose their directory.** Members of the animation
  archives whose names carry a directory are now stored under the bare
  file name, which is how scripts and the skeleton files name them. Tick
  3 needed this to find its clip's samples.
- **Create New Object Instance mode 8** (0x44e0c8, the jump table at
  0x44e92e, `primitives/create.rs`) is ported for an ordinary object,
  with no flag or only flag 0x20: the object is made where the object a
  local names stands, tested with the place test, removed again if it
  does not fit, and put there. Turning it (flag 0x20, when its facing
  differs) is not ported and fails loudly, as do a new person and an
  object of a group. This is only reached in tick 6, so it is
  **unverified**.
- **Object 15's dialog** in tick 2 is answered with 3, the second
  button, as the capture's events show. `sims boot` needs `--answers 3`
  to get past it.

How it was checked:

- Each of ticks 2 to 5 was compared with the capture's state at the
  return of the same lot update, outside the repository, with a script
  that builds our state from the install and the house file alone. One
  line per thing compared. Equal, after each tick: the tick count; the
  lot's and the game's random streams; the scheduler cursor; the funds;
  all 64 globals; the removal queue; the registration order; the set of
  objects; per object its GUID, data words, temps, attributes, scheduler
  word, last result, frames with their saved primitive state,
  relationships, slot count, position and two marks; per person the 256
  person data words, motives and previous motives bit for bit, the
  running and last actions, the queue and the objects in use; the
  people's animators, in every field we keep that the capture's memory
  pages hold (100 readings after ticks 3 to 5, 72 after tick 2; 23 fall
  on pages the capture lacks); and, read from the neighborhood file the
  capture's game read, the 463 neighborhood records, the inventories and
  registry byte 0x12.
- The tick-1 comparison is unchanged: equal except the C runtime stream
  and registry byte 0x12, which our tick 2 sets and the capture already
  had set after tick 1.
- The comparison of the load with its trace still gives the three known
  differences, all in object 19's frames.
- Synthetic tests cover the spiral's order, wall crossing, the search
  staying in a room, flag 2, a refusal, the native errors, and mode 8's
  success, refusal and loud failures. The gated test runs ticks 1 to 5
  on the Goth house, pins the state after tick 5 and asserts the tick 6
  stop.

What did not match, and why:

- The C runtime stream is 52 draws further on in the capture after every
  tick. Neither side draws from it in ticks 1 to 5: ours stays at the
  same value and so does the capture's. The likely cause is that the
  capture's game drew 52 more numbers before its load; that is
  **unverified**.
- Read from the install's neighborhood file, records 47, 48, 301, 344
  and 351 and record 301's inventory differ after every tick, because
  the install's file differs from the one the capture's game read.
- Registry byte 0x12 differs after tick 1 only (ours 0, the capture 1).
  What sets it before tick 2 in the original is not known.

What is **unverified**:

- Global 0x6e4ad4, which would make the search refuse more tiles, is
  taken to be 0; the capture does not hold it.
- Everything after tick 5, including Create New Object Instance mode 8.
- The paths of Find Location For and the place test that the Goth house
  does not reach. They fail loudly where they are not ported; the
  ported ones beyond what ticks 2 to 5 use are checked only by the
  synthetic tests against our reading of the code.

Tick 6 starts at tick count 88,449. Object 382 asks for a new object of
type 0x44A86A6C, a person, where the pedestrian portal (object 6)
stands. Making and placing a new person is not ported, so the tick
fails there, before anything is made. What would reopen this: a capture
past tick 5, or a difference found in a later tick.

## 2026-11-14: A person's animator is held, and tick 2 runs to Find Location For

Each person now has an animator in the simulation state. Animate Sim
and the animator's tick are ported as far as the Goth house reaches
them. Tick 2 gets past Animate Sim on object 16 and Stop All Sounds, and
stops at Find Location For (primitive 16) on object 364. K stays 1.

Some terms. The *animator* is the object at person +0xadc (vtable
0x671ee4) that plays a person's animations. A *clip* is one named
animation from the game's animation files. A *layer* is one clip
playing on the person's skin, with its own clock. The animator has
three *slots* for layers: the *foreground* (+0xc), the clip a script
plays; the *base* (+0x14), the clip of the person's posture; and the
*carry* (+0x13c), the clip of something carried. An *event* is a number
a clip sends to the script that plays it. *Presentation* is what only
the picture or the sound uses.

What was decided:

- **What is simulation state.** `World::animators`
  (`sims_sim::animator`) keeps, for each person whose animator has
  ticked or played: the animator's clock, its speed and the speed it
  moves towards, its state, the result of its last route, its position,
  direction and destination, the head's clock, the posture and carry
  bookkeeping, whether the base slot holds a layer, Animate Sim's
  progress through a clip's events, the event queue, and the foreground
  layer's clock, speed, phase, duration and event times. Scripts read
  these through Animate Sim (whether a clip is done, which event came),
  and the animator's tick reads them to decide what it writes to the
  person. Field offsets are the original's and are listed in the doc
  comments.
- **What is presentation**, and left out: the posed bones and the skin
  (0x5994c3, 0x5a15bc, 0x5a0ef5), the head's turn past the countdown
  it keeps in person words 42 and 45 (0x590768), the draw state
  (0x434101), the face (0x58faf6), where a carried object is drawn
  (0x590b27 and the tail of 0x58dc72), the base layer's own clock, and
  sound. The evidence is the original's code: none of these routines
  writes anything a script or the scheduler reads, as far as they were
  followed. The base layer's clip sends no events, so its clock decides
  nothing. That nothing the simulation reads depends on these is
  **unverified** beyond what the capture shows below.
- **The animator before the first tick** (`animator::LOADED`) is
  inferred. The capture's first look at an animator is after tick 1.
  Each value is the one tick 1 leaves alone, or the one that tick 1
  turns into what the capture holds (the doc comment gives the reason
  for each). For example, the speed starts at 1.0, because tick 1 raises
  it by at most 0.2 and the capture holds 1.2.
- **Animate Sim** (opcode 44, 0x58950b, through the person's dispatcher
  0x45cbb3): starting a clip on the foreground slot, the event queue,
  numbered events in order, an interrupt ending the clip, and the events
  a finished clip did not send. The clip's name comes from the global
  file's string table 156 (adult) or 157 (age 17 or less), and its
  duration and frame count from the animation files
  (`sims_content::Animations`).
- **The animator's tick** (0x58dc72), run from the person's tick after
  the decay: the clock (50 per tick, 42 when slowed), the carry choice,
  the base layer's bookkeeping, the speed (towards 0.2 times a base
  plus 1.2, up by at most 0.2 per tick), placing a standing animator on
  its person's tile, the foreground layer's clock (0x597c4c), and the
  head's countdown. All arithmetic is in single precision, as the
  original's.
- **Stop All Sounds** (opcode 48, 0x45253a) takes the true branch and
  changes nothing. Its only effect is a command to the sound mixer.
- **Resetting a person whose animator has ticked fails loudly.** The
  reset also resets the animator (0x589220), which is not ported. The
  resets seen so far are all at the load, before any animator ticks.
- **Snapshot format 11.** `World` gained the animators. The pinned
  states in `tests/resize.rs` and the synthetic world's digest changed.
  The tests strip the animators (`format_10_bytes`) and check that the
  rest of the encoding still gives the format-10 pins. The load's state
  has no animators. After tick 1 the rest of the state is the same as
  before, so the tick-1 pins for formats 9 and 10 still hold.

How it was checked. The private capture of the original running ticks 2
to 5 (the entries below) includes memory pages that hold the three
people's animators and their foreground layers. The scratchpad tool
`tick_verify.py` now reads them and compares each field we keep, as
bits for the floats.

Equal to the original:

- After tick 1: every animator field that the capture's pages hold, 58
  of 58 for people 16, 18 and 21. This includes the speed (1.2, bits
  0x3f99999a), the target speed (bits 0x3fb33334), the state, the
  position and the clock. 23 field readings fall on pages the capture
  did not save. The rest of our state after tick 1 is unchanged and
  still equals the capture, as the entry below lists.
- Where tick 2 stops (at object 364, after every lower ID has run):
  72 of 72 readable animator fields, including object 16's foreground
  layer (priority, clock, duration, speed, phase, event times, frame
  count, end mode, and whether it may be interrupted). 354 of the 355
  objects below 364 equal the capture after tick 2 in data words and
  frames, and the three people also in person data and motives. These
  are compared with the capture after the whole tick, so something later
  in the tick could still change them; the capture shows it does not.

Consistent only with the old prototype's isolated comparisons: nothing.
The old prototype's animation clock is marked approximate, so it was
not used as a guide.

Did not match:

- Object 262, which object 364 makes again just before the stop,
  differs in data word 29: ours -5, the capture's 0. Object 364 is about
  to place it with Find Location For, which is where we stop. That the
  placement writes the word is likely and **unverified**.
- Object 364's own frames, since it has not finished its turn.
- Everything else the entries below list as not matching still does
  not.

**Unverified**, with no evidence either way:

- That every motion of a clip binds to a bone of the person's skeleton.
  A clip with motions is taken to bind at least one.
- Which animation files the original's library loads, in what order,
  and whether it ignores case in a clip's name. A clip that two files
  define differently, or that only a name in another case matches,
  fails loudly.
- The base layer's clock, which is not kept.
- The other skin routine (0x5a0ffd). Either routine updates the layers
  once per tick; which one runs is taken to change only the pose.
- Removing a finished layer from the skin (+0x64), which nothing ported
  reaches.
- The head's paths when bit 0 of person word 42 is set; they fail
  loudly.

What fails loudly: Animate Sim on an object that is not a person, on
the base or carry slot, with clip 0, with a name from anywhere but the
global tables, with a missing clip or one with timed events, replacing a
foreground layer, and at the end of a clip (0x592904); the animator's
tick in any state but 0 and 3, when it has to turn or walk, when it
starts a carry or posture clip, and on a layer end mode other than 0 or
a fading layer; and resetting a person whose animator has ticked.

Found wrong in existing code or docs: removing a person does not drop
its entries in `World::person_ticks` or `World::animators`. Nothing
reads them for a removed person, and no person is removed in the
ticks run, so it is noted and left.

What tick 2 needs next: Find Location For (0x44b3c8). Mode 0 searches
for a free place near the reference object (0x4347f0, which draws
random numbers through 0x46f457), tests each place (0x432679,
0x4329c1) and puts the object there. In the capture, object 262 ends
tick 2 at (632, 344, level 1). After that, tick 3 has people 18 and 21
start a clip, and an Animate Sim clip's end (0x592904) will be needed.

## 2026-11-13: The neighborhood's people are held, and tick 2 runs to Animate Sim

The simulation now holds the neighborhood's people the way the original
does while a lot runs. Tick 2 of the Goth house gets past the scripts
that read and write them, and stops at Animate Sim on object 16, which
needs a person's animator. K stays 1.

Some terms. A *record* is the original's 628-byte entry for one person
of the neighborhood, kept in a list in the neighborhood object (+0x148):
an ID, the GUID of the person's type, a revision, 256 words of *person
data*, *relationship rows* (one per other person, a list of numbers
each), a cached friend count and a mark that says the count is out of
date (the *stale* mark). A record's *slot* is its place in that list. A
neighbor's *inventory* is a list of tokens (kind, GUID, count). The
*frame storage* is the block of words in which the original keeps an
object's script frames; a new frame's locals start with whatever an
earlier frame left there.

What was decided:

- **The records live in `World::neighbors`** (`sims_sim::neighbors`):
  every slot, empty ones included, with the inventories and registry
  byte 0x12, which a relationship write sets. `World::read_neighbors`
  builds them before the load, as the reader (0x42feb2) and the
  neighborhood's reload (0x424806) do: the file's records as read, then
  person data word 61 (the family) rebuilt from the families in their
  sorted order (0x425209, 0x425244). Ported from the old prototype's
  `prepare_family_reload` and `rebuild_family_links`
  (legacy/crates/sims-vm/src/neighborhood.rs). A person type with no
  record fails loudly: the reload would make one, which is not ported.
  `sims_content::Neighborhood` now keeps the empty slots, since a script
  names a record by its slot plus one (0x4256f4), and reads the
  inventories from the NGBH chunk (0x4254f6). Constructing a person and
  attaching it to its record now read these records, not a copy in
  `People`.
- **What reads and writes them**, each ported from the original's code
  named in its doc comment: Set To Next modes 5 and 7 (0x425b15,
  0x44ed07); expression scope 32, a record's person data (0x4537e3);
  Relationship with flag 2, and between two people on the lot, which
  keep their rows in the records (0x44d17c; the row's key is the target
  record's own ID; a write marks both records stale and sets the
  registry byte, 0x44485b); Manage Inventory operation 3 (0x4552f8).
- **Idle For Input** (opcode 17, 0x45db94), through the person's own
  dispatcher (0x45cbb3), with the test of whether something interrupts
  the wait (0x45d7c0: a queued action of higher priority or pushed to the
  front, or a very low motive on a residential lot). Checked against the
  old prototype's `idle_for_input` and `should_interrupt`
  (legacy/crates/sims-vm/src/action.rs). Starting the queued action is
  not ported and fails loudly.
- **Get Distance To** (opcode 11, 0x451f27, distance 0x434148): the sum
  of squares in 32-bit integers, then single precision for the rest, and
  truncation toward zero. The prototype has no version of it.
- **A returning frame leaves its return target in its node word.** The
  interpreter wrote the target of every branch into the frame's node
  word, except a return. The original writes it for a return too (0xfe or
  0xff, at 0x474cea and 0x474d1b) before it pops the frame, so the frame
  storage keeps it. This was wrong in the existing code. The evidence: in
  the capture's tick 2, a person's new frame finds 254 in a local that
  lands on such a word; ours found the old node, 15.
- **Snapshot format 10.** `World` gained the records. The pinned states
  in `tests/resize.rs` and the synthetic world's digest changed, for two
  reasons. The records: the tests strip them (`format_9_bytes`) and check
  the remaining encoding. The return targets: before the pins changed,
  every pinned state was dumped under the old and the new interpreter
  and compared outside the repository. Everything but the frame storage
  was equal. In the frame storage, every word that differed sits at a
  frame's node word and now holds 0xfe or 0xff. The tick-1 gated test
  still equals the capture.

How it was checked. The private capture of the original running ticks 2
to 5 (the one in the entry below) holds the 463 records, the
inventories and the registry byte after every tick. The scratchpad tool
`tick_verify.py` now compares them too, one line per field.

The capture's game read another neighborhood file. Its records equal
ours read from the emulator guests' `Neighborhood.iff` (private, outside
the repository). The
install's file differs from that one in five records: 47 and 48
(revision, person data, relationship rows of three numbers instead of
two), 301 (revision, person data, and an inventory), 344 and 351
(person data). So the comparison is run both ways: with the guests' file
every record field and the inventories are equal after tick 1; with the
install's file only those five records and the inventory differ.
Everything else in our state, after tick 1 and where tick 2 stops, is
the same from either file. The gated tests use the install's file and
say so.

Equal to the original:

- After tick 1, all 463 records (ID, GUID, revision, person data,
  relationship rows, friend count, stale mark) and the inventories, read
  from the same file. Before this, the load trace had shown family word
  61 of every record equal to ours.
- Where tick 2 stops: object 16's frames (trees, nodes, stack objects,
  arguments, locals) equal the capture's after tick 2. The capture has
  object 16 at that same node after ticks 3, 4 and 5 as well.
- Part way through tick 2, at that stop, records 32, 33 and 34 equal the
  capture's after tick 2 except two rows (33 to 34, and 34 to 33), which
  in the capture gain a third number later in the tick. That is a
  spike's comparison of a state taken part way through the tick, not a
  like-for-like one, so it is not counted.

Consistent only with the old prototype's isolated comparisons: the
family rebuild and Idle For Input's paths, which the prototype has in
the same form.

Did not match: registry byte 0x12 is 1 in the capture after tick 1 and 0
here. A load trace of another run shows it 0 at the resize and 1 after
the object read; what sets it there is not known, and nothing ported
reads it. The C runtime's random stream is still 52 draws further on in
the capture (the entry below); ours does not move in tick 2 up to the
stop.
The six type attribute words no file holds (the entry two below) play no
part in these results: the comparison is the same with and without them.

**Unverified**, with no evidence either way:

- Get Distance To's value. It is ported from the code alone; the
  comparisons that cover it are whole ticks, and tick 2 does not finish.
- That the original keeps nothing else in the records that a lot
  changes. The capture's records change only in the rows named above
  through tick 5.
- The family friend count a relationship write also marks (0x4246b9,
  family +0x124). The families are not kept; nothing ported reads it.
- Manage Inventory's token layout (12 bytes: kind, GUID, count), read
  from the code; every inventory in the capture is empty.
- The interrupt test's paths: the capture does not show which way it
  went for the Goth house's people.

What fails loudly: Animate Sim (opcode 44), whose animator tick
(0x58dc72) moves the person and runs the animation clock; Idle For
Input starting a queued action (0x45dc14); Manage Inventory operations
other than 3; a person type with no record; a record whose GUID has no
definition, in Set To Next mode 7 and Manage Inventory; Set To Next mode
11; an empty slot in the file's inventory list; and everything the entry
below lists.

What tick 2 needs next: Animate Sim and enough of the person's animator
to say, each tick, which events an animation reached and whether it is
done, from the original's code (0x58950b, 0x58dc72). The old prototype's
animation clock is marked approximate there, so it is no guide. With it
tick 2 should reach what the capture shows next: object 15's two-button
dialog, answered with the second button, and object 262 made again.

## 2026-11-13: Ticks run, and the first one matches the original

`sims boot <install> <house> --ticks N` now runs N ticks after the load
and prints the hash. On the Goth house tick 1 runs and our state after
it equals the original's. Tick 2 stops at something not ported, and the
boot fails and names it and the tick.

Some terms. A *tick* is one lot update, the original's 0x46d345. The
*scheduler* is the part of it that goes over the objects. An object's
*countdown* is the low half of its scheduler word: while it is above 0
the scheduler counts it down instead of running the object. A *dialog
record* is what the Dialog primitive leaves for the game to show; its
*status* says whether it was shown and which button answered it. A
*modal* dialog stops the game until it is answered.

What was decided:

- **The lot update, the scheduler pass, the object tick and the person
  tick are ported** as `Interpreter::lot_update`, `scheduler_pass`,
  `tick_object` and `tick_person` (`crates/sims-vm/src/tick.rs`), read
  from 0x46d345, 0x46d553, 0x4521f7 and 0x45cc8c.
- **The end of the lot update is more than the removals.** The roadmap
  said 0x4412d1 carries out the queued removals. It first goes over the
  dialog records, showing new ones and destroying finished ones, then the
  game's special panels, and only then, after a full pass, the removals;
  it ends with a check that a household Sim is selected. Ported as
  `Interpreter::end_of_update`, without the panels, which no ported
  primitive opens.
- **Dialog is ported in the form the simulation needs**
  (`primitives/dialog.rs`, 0x451552): records in `World::dialogs`, the
  statuses 0 to 5, the true branch for the first button and the false
  branch for the second and third. What a dialog says and shows is not
  modeled. Kinds 3 (text) and 5 to 15 (the special panels) fail loudly.
- **The player answers modal dialogs inside the end of the update.** The
  original waits there, in the dialog's own loop, until the dialog is
  answered. `sims_vm::Player` gives the answer. `sims boot` answers a
  one-button dialog with its only button, and a dialog with a choice
  with the next value of `--answers`; with none left the tick fails. A
  dialog that does not stop the game fails loudly.
- **Motives** (`sims_sim::motives`): the decay (0x420fb6), the mood
  (0x45d165) and the motive changes (0x45cdb3), all in single precision.
  The tuning comes from the global file (`Program::motive_tuning`).
- **Snapshot format 9.** `World` gained the dialog records with the
  registry's word 0x100, and each person's tick state: the tick of the
  last decay (person field 0x810) and the room last seen in (0xafc).
  Before a tick these encode as three zero bytes at the end. The gated
  tests check every state pinned under format 8 by dropping those three
  bytes (`format_8_bytes` in `tests/resize.rs`), and the synthetic
  world's old digest the same way.

How it was checked. A second run of the original on the same install and
house is private and outside the repository. Its observer attached while
the game sat in the dialog loop of its first tick. It holds the full
state at the entry of the next lot update, which is the state after tick
1, and the states after ticks 2 to 5, with a log of the dialog functions
it passed. The comparison is the scratchpad tool `tick_verify.py`.

Equal to the original after tick 1: the tick count, the lot's and the
game's random streams, the scheduler cursor, the funds, all 64 globals,
the removal queue (empty), the registration order, the set of objects
(the 7 IDs the load queued and 2 the tick queued are gone, in both), and
for each of the 377 objects its GUID, 72 data words, temps, attributes,
scheduler word, last result, frames with their saved primitive state,
relationships, slot count, position, and the removal and selected marks;
for each of the three people its 256 person data words, its 16 motives
and 16 previous motives bit for bit, its running and last actions, its
queue and its number of objects in use. Of the 31 lines compared, 30 are
equal. The original's log agrees with the dialogs: in tick 2 the five
controllers acknowledge an answered record and go on, and in tick 3 object
15 acknowledges a record answered with the second button.

Did not match: the C runtime's random stream. The capture's state is 52
draws further on than ours, after tick 1 and after every later tick (it
never moves in that run's ticks 2 to 5). Our tick 1 draws none. The trace
of the load, from another run, equals ours at the start of tick 1. The
likely cause is that this run's game drew 52 more numbers before its
load; that is **unverified**, and so is that the original's tick 1 draws
none.

Consistent only with the old prototype's isolated comparisons: the mood
curves' parsing (legacy `Curve::parse`, compared there with 0x461423 and
0x46137b run alone). The decay and the mood themselves are equal to the
original (above), for the Goth house's people at this one decay.

**Unverified**, with no evidence either way:

- What an object's animated sprites, a person's animator and the graphics
  work every tenth tick do; they are taken as presentation and not
  modeled. The same for signal 0xdb (only the interface takes it) and
  signal 0xf7 (only the house viewer); the lot's second subscriber, at lot
  0x10, which sees every signal first, is not known.
- The instruction limit of one tick, 458,605 (read from a memory image).
- That making a dialog record runs no object code.
- Kind 2 (three-button) dialogs, an informing dialog, and the second and
  third buttons' effect on temps and registry word 0x100: read from the
  code, not exercised by the capture.
- The scheduler pass stops at the highest ID held; the original's table
  can be longer. It shows only for an object made during a pass with a
  higher ID.

What fails loudly: the hour's rollover; a scheduler tick that returns
false; a thread that returns from its outermost frame or runs past the
limit; a person entering or leaving an indoor room; the lot service every
tenth tick (0x4165e3; first at tick count 88,450 on the Goth house);
person word 1; removing a tile of a multi-tile object or an object that
owns a dialog at the end of the update; no selected Sim; dialog kinds not
ported; a choice with no answer.

What tick 2 needs: Set To Next mode 5, over the neighborhood's people.
Object 16, a person, reaches it in a tree of the people's semi-global
file that also reads those people's relationships. Porting it means
holding the neighborhood's people (their records, person data and
relationships) in the simulation. That would reopen this entry's K = 1.

## 2026-11-13: The load runs to the first tick, and rooms have light and scores

`sims boot` now runs all 22 load steps on the Goth house. The state it
ends in is the one the original's first tick starts from. Ticks are not
implemented.

Some terms. The *selected Sim* is the one the player controls; the
original marks it with a flag on the object and writes its ID to global
3. An *indicator* is one of two objects the game keeps on every lot; the
first is the marker over the selected Sim. A room is *marked* when it
waits to be counted. A level *waits* when the lot knows that one of its
rooms is marked. The *room update* counts the marked rooms.

What was decided:

- **Step 16 clears global 3** (`Interpreter::announce_house`, 0x44302e).
  When the house is announced and no Sim is selected, the registry asks
  for the indicators and then announces that nobody is selected, which
  clears global 3. The roadmap had this as part of step 17.
- **Which Sim is selected** (`Interpreter::select_resident`, the end of
  0x4f1ae9). A selected Sim that belongs to a family stays. Otherwise it
  is the family's first member: the first object of that member's type.
  Whether it may be selected depends on the lot's zoning and number and
  on the Sim's visitor word (0x4e989b). The roadmap had asked whether it
  is the Sim the house saved in global 3 or the family's first member. It
  is the first member: global 3 is cleared before the choice, and no
  save holds the Sim's mark.
- **Selecting a Sim** (`Interpreter::select`, 0x445449): the mark
  (`Object::selected`), the indicators (0x44664c), then global 3.
- **An indicator that leaves the lot is taken off twice.** When an
  object leaves the lot the registry is told (0x447d2b), and it takes an
  indicator off once more (0x4481eb). So the indicator is asked function
  10 twice. `placement.rs` used to say that telling the registry changes
  nothing the simulation holds.
- **Step 19 is `Interpreter::go_live`**: every room is marked, the
  lot's statistics are worked out, and global 19 becomes 0. Of the
  statistics (0x416085) only globals 23 and 37 are ported.
- **Steps 21 and 22 are `Interpreter::before_frame`**: the rooms that
  wait are counted, and global 12 becomes 0.
- **The room update is ported whole** (`World::update_rooms`, 0x466c3c):
  the objects standing in each room, the room's light and score, the
  lot's outdoor score, and the rule that a room whose light changed is
  marked again. `Rooms` now holds which levels wait.
- **Step 10a is two updates.** The original counts the rooms after step
  10, then marks them all when the house read ends (signal 0xdd) and
  counts what waits. Before, one count gave the same sizes. With light
  it does not give the same marks: the first count changes every room's
  light, which marks every room again.
- **The room arithmetic rounds every operation to single precision**
  (`Ext::single` in `sims-core`). See below.
- **The snapshot format is 8.** New: each object's selected mark, each
  room's counts, light, score and pool mark, and the rooms' waiting
  levels, outdoor sum and outdoor score.

How it was checked. The private trace of the original loading the Goth
house stops at the entry and return of the funds setter, of the
selection (twice), of the mode change (twice) and of the lot's part in
it, at the entry and return of the wait check, and at the entry of the
first tick. It holds every call of the function that runs an object's
callback (0x422d46). Our run was compared with it outside the repository
(`live_verify.py` in the private scratchpad, one line for each thing
compared).

*Equal to the trace:*

- **The whole state at seven stops**: the entry of step 11 (after the
  room count), the entry of the funds setter (after step 16), the return
  of the selection (after step 18), the return of the lot's part of the
  mode change and the entry of the second selection (inside and after
  step 19), the entry of the wait check (after step 21), and the entry
  of the first tick. At each: everything the entry below lists for the
  end of step 13, and each object's selected mark.
- **The rooms at each of those stops**, all 11: the twelve counts, the
  lights and people, the score and the light bit for bit, whether the
  room is outdoors, whether it is a pool, the mark, the levels that
  wait, and the lot's outdoor sum and score bit for bit.
- **The callbacks of step 18**, in order: 383:10, 383:10, 383:9, 21:9.
  No other callback is asked between the end of step 13 and the first
  tick, there or here.
- **Steps 13 to 15** as before; the comparison of the entry below was
  run again with the room fields added.

Why the six indoor rooms stay marked after step 21, which the roadmap
could not explain: their light changed. At step 10a every indoor room
was fully lit through its windows. Step 13 reset the windows, which
cleared the light each lets in. So at step 21 the indoor rooms come out
dark, or lit by their lamps only, and a room whose light changed is
marked again. The outdoor rooms have the daylight both times.

The original's FPU and single precision. One room's score in the trace
is 3 units in the last place away from what `sims-core`'s 64-bit
register model gives. It is exactly what the sum gives when every
addition is rounded to 24 bits. The other ten rooms are equal either
way. So the FPU's precision control is set to single precision when the
room update runs, as Direct3D sets it. The room code now rounds each
operation that way, and all 11 scores, both times, are equal bit for
bit. Motive values compared earlier were equal under the 64-bit model.
The sums that made them may not tell the two apart. That has to be
looked at again when motives change in a tick.

The second 383:10. The trace shows function 10 asked of the indicator
twice, and the code that moves the indicators asks once. To find the
second caller, the original's selection code (0x445449) was run in an
emulator on the memory the trace holds at its entry. It ran through both
calls and showed the chain: remove from lot (0x433125), told to the
registry (0x447d2b), which removes an indicator again (0x4481eb). The
emulation is weaker evidence than the trace, and only the caller rests
on it. The order of the four callbacks is equal to the trace.

*Consistent with the old prototype's isolated comparisons only:*

- The room update on a lot with a pool, and an object's share when it
  is hidden, has a negative room word, or is one of the two categories.
  The prototype compared the score arithmetic and the object visit with
  the original's code run alone. The Goth house has no pool.
- The names and defaults of the room constants. The Complete Collection
  has the constants the code defaults to.
- The zoning rules of 0x4e989b for other lots than a residential one.
  They were read from the code here; the prototype read the same.

What changed in earlier work, and why:

- **`World::order` and `World::scheduler_cursor` were documented
  wrongly.** The comments said the scheduler visits the registration
  list and that the cursor is a place in it. The scheduler's pass
  (0x46d553) goes through the registry's table of objects by ID, the
  table the ID lookup (0x40743b) reads, and the cursor is an ID. Only
  the comments changed; nothing uses either yet.
- **`World::update_rooms` takes the constants, the level and the
  objects on a tile.** `sims-vm` supplies them
  (`Interpreter::update_rooms`). The room tests in `sims-sim` count with
  no objects.
- **The gated tests pin new digests**, because the encoding changed.
  Each state that was pinned under format 7 is also encoded without the
  new fields (`format_7_bytes` in the test file), and that encoding
  still has the digest pinned before. So the states are the same ones.
  The chain back to formats 6, 5 and 4 now starts from that encoding.
- **The gated test of step 13 no longer says steps 14 to 16 change
  nothing.** Step 16 clears global 3. A new gated test takes the load
  from there to the first tick.
- **The roadmap's step 17 was "clear the selected Sim and set the funds
  again".** The clearing is step 16's. Step 17 is the funds.

What is **unverified**:

- **Selection on any lot but this one**: a family whose first member is
  not on the lot or may not be selected, a visitor, a community lot.
- **Three things the interface holds**, taken as the trace shows them:
  every family member is on the list of portraits; the switch that lets
  any Sim be selected (0x6e4f64) is off; a person's slot 1 takes an
  indicator. The trace's memory shows that slot flag on the two Goths
  whose slots it holds.
- **The order of the original's list of people**, which decides who is
  selected if two people have the mark. The registration order is used.
- **The statistics pass beyond globals 23 and 37**: global 22, the
  value of the lot's objects and architecture, two flags of the family's
  record. They are left as they are. In the trace they do not change,
  but they already hold what the house saved.
- **The bits of global 37 other than bit 0.** The Goth lot sets only
  that one. The others are read from the code.
- **The daylight's multiplier** (0x6f015c), which is 1.0 in the trace's
  memory and is taken to be 1.0 always.
- **The level the first room update of step 10a counts.** It is taken
  to be every level. The second update counts every level whatever the
  first did.
- **The time of day other than day**, and the lot's outdoor score at
  night, dawn and dusk. The trace is by day.
- **Single precision outside the room update.** See above.

What is not modeled: the indicator's and the Sim's graphics during the
selection; the star markers of the Superstar pack, which the original
handles only for a Sim that is not selected; the lights the room update
places in a room and the lighting of the tiles; the rooms' cached route
rectangles; what a person is told when the mode changes; the count of
the family's friends; and a field of the family's record that the end
of the load clears.

What fails loudly: deselecting a Sim; selecting a Sim with a date, or
one whose slot 1 holds something, or one of several tiles; a family
member's type that two definitions name; a star marker leaving the lot;
an indoor room that is dark, has no lights and has people in it, where
the light it shows depends on the room knowing it is occupied
(0x468802), which is not ported; and `sims boot --ticks` with a number
above 0.

About the first tick. It is not implemented, because one tick could not
be both built and checked in this piece of work. The roadmap now says
what a tick is in the original (0x46d345 and 0x46d553, read from the
code), what the first one needs, and what there is to check it against.
The main finding: an earlier capture of another run of the original, at
tick 88,445, differs from our baseline only in what one tick would
change, in every object field compared, and has the same two game
random streams. So the first five ticks can be checked against it.

What would reopen this: a house with a pool, or loaded at night; a lot
whose people stand indoors in the dark; a house where the selection is
not the family's first member; a trace that shows the statistics pass
writing a value; any float result that single precision does not give.

## 2026-11-13: People are visited like the original visits them, and the load runs until the interface takes over

Load step 13 visits every object on the lot. It used to stop at the first
person. `sims boot` now visits people too, finishes the step, and runs
steps 14 to 16. On the Goth house all 382 visits run. The boot stops
after step 16, before the original's interface selects a Sim.

Some terms. A person's *neighborhood record* is its entry in the
neighborhood file: an ID, the person's type, and 256 person data words.
A record has a *revision*, which says how many of those words it keeps
for the person. A type has *changed* when the scripts installed now
differ from the ones the house was saved with. Function 3 of an object is
its Reset tree and function 0 its Init tree.

What was decided:

- **A person's visit is `Interpreter::visit` too** (0x459bf6). It differs
  from an object's in three ways. The person first takes the fields its
  neighborhood record keeps (`attach_neighbor`, 0x4258a3). It is not
  asked its Load function. And if its type changed it resets in its own
  way (`reset_person`, 0x45e791).
- **The record's fields.** The record's ID goes into person data word 31.
  Then the words `persistent_fields` lists for the record's revision are
  copied from the record: 29 at revision 1, 48 at revision 10. The record
  is the first of the person's type in the file.
- **A person's reset**, in this order: person data word 30 is cleared;
  the gender, age and skin tone are read again from the body strings of
  the person's type (`reload_profile`); the thread restarts at Main and
  then the Reset tree runs on top of it; person data words 0, 40, 59, 33,
  71, 72 and 73 and data words 34 and 9 are cleared, with any motive
  changes in progress; and each queued action is dropped, its target
  being asked function 4. An object's reset runs the Reset tree first and
  restarts the thread afterwards. A person's does it the other way round.
- **Only a person that is doing nothing can be reset.** A person that is
  using an object, has a running action with a target, is in the middle
  of a route, is inside another object or off the lot, or has person data
  word 73 set fails. The original does more for each. No person of the
  Goth house is in any of those states.
- **The cached room and the motive curves are not kept.** After the
  visit the original notes the person's room, which it uses to light the
  room, and loads the curves that turn each motive into its share of the
  mood. The lighting is not modeled. Nothing reads the curves until a
  person's motives are updated in a tick. `curve_table` says which string
  table the curves come from. So `World` did not change, and the snapshot
  format is still 7.
- **`People` now carries each person type's profile**: three of its body
  strings (gender, age, skin tone) and the skin tone names from its
  semi-global file.
- **Step 14 is `Interpreter::ensure_indicators`** (0x445e4a): create the
  two indicators unless the lot has them. Step 16 calls it again.
- **Step 15 is `World::finish_selection`.** After the visits the function
  that selected the household may delete people who do not belong, create
  a tutorial controller, reset the lot, and work out who is a visitor.
  None of that is ported. The step checks that none of it could do
  anything, and then changes nothing: every person is of the selected
  family and not a visitor, global 26 is 0, and the family's record does
  not ask for a reset (flag 4). Otherwise it fails and says which.
- **The roadmap's step list was wrong at its end and is corrected.** It
  had one last step, "create the indicator objects and finish
  selection". The trace shows nine more stops after the visits. Steps 14
  to 16 belong to the loader and are ported. Steps 17 to 22 are the
  interface's: it clears and sets the selected Sim, moves an indicator
  into that Sim's slot, goes to live mode, and updates the rooms. They
  are written down in the roadmap from the trace and are not ported.

How it was checked. The private trace of the original loading the Goth
house holds a full state at the return of step 13, and at three stops
after it: the return of the indicators' creation after the last visit,
the return of the lighting update that ends the household selection, and
the return of the indicators' creation called when the house is
announced. For every visit it holds each call of the function that runs
an object's callback (0x422d46). Our run was compared with it outside
the repository (`people_verify.py` in the private scratchpad, which
prints one line for each thing compared).

*Equal to the trace:*

- **Every call of every visit.** For each of the 382 visits: the object
  visited, and the calls it makes, as object and function, in order.
  1,781 calls. The three people are visits 202 to 204, objects 21, 18
  and 16, and each is asked function 3 and then function 0.
- **The whole state at the return of step 13.** All 386 objects: GUID,
  the 72 data words, temps, attributes, scheduler word, last result,
  every frame's tree, node, stack object, parameters, locals and saved
  primitive state, the relationships, the number of slots, the position
  and the removal-without-resets mark. For the three people also the 256
  person data words, the 16 motives and the 16 previous motives bit for
  bit, the running action and the last action (ten fields each), the
  queued actions and the number of objects in use. The tick, the 64
  globals, both game random streams and the C runtime's, the funds, the
  removal queue, the indicator references, the registration order, the
  instance lists, which rooms wait to be counted (8 of 11), and the
  arrays inside the lot's size, the object grids included.
- **The same at the three later stops**, after our steps 14, 15 and 16.
  The trace changes nothing compared between them, and neither do we.
- **The two stops in the middle of visit 140**, as in the entry below,
  now with the people's motives and actions compared as well.

This is the first check of what is not an object field after visit 140.
The entry below called that unverified: the global random stream, the
globals, the arrays and object grids, the removal queue and the rooms
waiting to be counted. All are equal at the step's return. The global
random stream moves during the step, so its being equal is not empty.

What the trace shows of a person's visit, and what it cannot show:

- It shows the record's fields being taken: person data words 46 to 55
  change, to the record's values. The other words copied already hold
  the record's values.
- It shows the thread restarting: three frames become one Main frame at
  node 0 with no stack object.
- It shows what the Reset and Init trees do: the last result becomes
  true and seven motives change. Ours are equal bit for bit.
- It cannot show the words the reset clears or reloads being written.
  In this house they already hold the values written: the cleared words
  are 0, and the gender, age and skin tone read from the body strings
  are the saved ones. So the trace shows them unchanged, and so do we.
- It cannot show whether Main is restarted before or after the Reset
  tree runs. The result is the same here.

*Consistent with the old prototype's isolated comparisons only:*

- The order inside a person's reset, and which words it clears. The
  prototype ran the original's reset alone on 16 cases.
- The list of fields a record keeps, by revision. The prototype compared
  126 cases. The Goth records have revision 10.
- The body strings read by the reset and how they become numbers. The
  prototype compared 216 cases.
- The conditions of step 15, which the prototype compared on 192 cases.
- A queued action being dropped and its target asked function 4. No
  person of this house has a queued action.

What did not match, and why it does not matter. Object 19's second frame
has tree ID 0x8118 in the trace and 0x118 here and in the house file, at
every stop before its visit. Its visit is number 206. The visit makes
the same three calls here as in the trace (functions 2, 3 and 0), and at
the step's return object 19 is equal in every field compared: its reset
replaced the thread. So the bit does not change what the visit does.
What sets it is still not known. Six type attribute words belong to the
game that was running; with those copied from the trace's start they are
equal.

What changed in earlier work, and why:

- **The gated test of step 13 now runs the whole step.** It still shows
  the state pinned after visit 201: it makes the first 201 visits one at
  a time on a copy and gets the old digest. No other pinned digest
  changed, because `World`'s encoding did not.
- **`sims boot` no longer says it stops before the indicators.** It
  reports steps 14 to 16 and stops before step 17.
- **The roadmap's "Progress" said the boot runs through step 12.** It had
  been running into step 13 since the entry below.

What is **unverified**:

- **A person that is doing something**, as listed above. It fails.
- **A neighborhood record of a revision below 1**, which holds no person
  data, and a person with no record. Both fail.
- **The motive changes in progress being cleared.** The trace does not
  hold them, and the Goths have none.
- **Which motive curves a person gets** (`curve_table`). Nothing loads
  them yet. The trace holds the two global tables at the end of the
  load, not which person uses which.
- **Step 15 on any other lot**: a visitor, a person of no family, the
  tutorial house, a family that asks for a reset. All fail.
- **Steps 17 to 22**, which are not ported. What the trace shows of them
  is in the roadmap. Two things about them are not known from the
  trace: how the game picks the Sim it selects, and what makes the six
  indoor rooms still wait to be counted after the room update.

What is not modeled: a person's animations, graphics, action icons and
accessories during a reset; the guard against a reset inside a reset;
the cached room and the lighting it drives; the lighting update at the
end of step 15; and the signal that announces the house, beyond the
indicators it asks for.

What fails loudly: resetting a person in any of the states above; a
person type with no profile or no neighborhood record; a record with no
person data; a body string age that overflows 32 bits; and each
condition of step 15.

What would reopen this: a house whose people are in the middle of
something when saved; a house with visitors; a capture that holds a
person's motive changes or shows which curves it uses; anything that
shows a reset writing a value that differs from what the trees or the
save left.

## 2026-11-13: Objects are visited up to the first person, and frames keep what earlier frames left

After the household is selected the original visits every object on the
lot, in registration order (load step 13). `sims boot` now makes those
visits for objects that are not people, and stops at the first person. On
the Goth house that is 201 of 382 visits.

Some terms. An object's *callback table* lists the trees the game runs on
set occasions; "function 2" is the second entry counting from 0. Function
0 is Init, 2 is Load, 3 is Reset, 9 is run when the object has been placed
and 10 before it leaves. A type has *changed* when the scripts installed
now differ from the ones the house was saved with (step 7). To *rebuild*
an object is to take it off the lot, reset its storage, run its Init and
give it a first turn, and put it back.

What was decided:

- **A visit is `Interpreter::visit`** (0x431e2e). The object's position is
  written again, with its footprint and room, for it and everything it
  holds. Its Load function runs. If its type changed, it resets, and then
  Init runs. Two scheduler flags are refreshed. A reset runs the Reset
  function if the object has one, and otherwise rebuilds the object.
- **The step stops at a visit it cannot make.**
  `Interpreter::visit_objects` makes each visit on a copy of the lot and
  keeps the copy only if the visit finished. So the lot is always in a
  state the original is in between two visits, and the result says which
  visit failed and why. A person's visit fails: it needs the person's
  fields from the neighborhood record, the person's own reset, its room
  and its motive curves, none of which is ported.
- **Placement is `placement.rs`.** Taking an object off the lot and
  putting it on a tile or into another object keeps three things in step:
  the object grid, the slots, and each object's link words (data words 14
  and 26, and 2 and 3). The callbacks run in the original's order. A
  multi-tile object moves as a group.
- **The frame storage is state** (`World::frame_storage`). The original
  keeps a thread's frames in one block of memory, one after another, and
  does not clear the block. A new frame's locals are not zeroed: they hold
  what an earlier frame left at that place. We now keep that block for
  every object whose thread has pushed or popped a frame, write the live
  frames into it at each push and pop, and take a new frame's locals from
  it. Before the original runs a callback it writes an eight-word mark
  above the top frame; we write it too, because it moves where the
  callback's frames lie. Two words of each frame's header hold a pointer.
  A local that lands on one cannot be read until it is written; reading it
  fails and says so.
- **The lot's indicators are made in step 13** when the lot has none and
  an object comes onto it from outside (0x44752e calls 0x445e4a). In a
  load that is the first rebuilt object that stands on the lot.
- **An object with graphics of its own draws 32 numbers** from the C
  runtime's random stream when the factory makes it. The indicators are
  such objects. Only the draws are kept. This also settles the stream's
  state at the start of a load, which earlier entries called state of the
  running game that no file holds. In the trace the stream starts the
  load at its initial state 1 advanced by exactly 64 draws: the two
  indicators the game made when it started. `sims boot` makes those two
  on its starting lot, so the stream now equals the trace's at every stop
  from the files alone.
- **New primitives**, each for the cases the visits reach: Notify Stack
  Object out of Idle (49), Relationship (26) and Old Relationship (24),
  Create New Object Instance (42), Run Tree By Name (28), Set To Next
  modes 8 and 9, Refresh modes 1 and 2, and Generic Sims Call 10, 11 and
  43. What each leaves out fails with an error that names it.
- **The snapshot format is 7.** `World` has two new last fields: the list
  of fires and the frame storage.

How it was checked. The private trace of the original loading the Goth
house holds a full state at four moments of this step: its entry; the
entry and the return of the function that creates the indicators, which
runs in the middle of visit 140; and the step's return. For every visit
it also holds each call of the function that runs an object's callback
(0x422d46), with the object and the function's number. Our run was
compared with it outside the repository
(`visits_verify.py` in the private scratchpad).

*Equal to the trace:*

- **Every call of every visit that runs.** For each of the 201 visits:
  the object visited, and the calls it makes, as object and function, in
  order. 525 calls.
- **The whole state in the middle of visit 140**, at the entry of the
  indicators' creation. All 382 objects: GUID, the 72 data words, temps,
  attributes, scheduler word, last result, every frame's tree, node,
  stack object, parameters, locals and saved primitive state, the
  relationships, the number of slots, the position, the
  removal-without-resets mark, and the three people's data. Also the
  tick, the 64 globals, both game random streams, the funds, the removal
  queue (seven controllers asked to be removed in their Load trees), the
  registration order, the instance lists, which rooms wait to be counted
  (none), and the arrays inside the lot's size, the object grids
  included. By then 139 visits are over, 137 that only run Load and two
  that rebuild a controller off the lot, and visit 140 has taken the
  first object off the lot, rebuilt it and put it back.
- **The same when the indicators' creation returns**, with the two new
  indicators, and the C runtime's random stream, which is 64 draws
  further on. Object 246 is left out there, because the trace stops in
  the middle of its visit, and the global random stream, because the
  rest of that visit draws from it.
- **The 205 objects visited or created by visit 201**, against the
  trace's state at the step's return: every object field listed above.
  17 of them differ from the trace in data words 14 and 26 and in nothing
  else. In the trace each of the 17 is linked behind an object that is
  visited after visit 201. That object's rebuild takes it off the tile
  and puts it back in front, which rewrites those two words of the object
  behind it. So they are what they should be after visit 201.
- **The frame storage's effect.** The locals of restarted Main frames
  hold in the trace what our frame storage gives them: for example object
  12's one local is 280, the tree ID of the frame that used to lie where
  the local now does.

The comparison is not empty: between the step's entry and visit 140 the
trace changes 10 scheduler words, 10 attribute sets, the frames of 9
objects, global 36, the global random stream, the removal queue and an
instance list, and all of those are equal here.

*Consistent with the old prototype's isolated comparisons only.* These
run on the Goth house too, but the trace cannot show them wrong or right
in detail, or they do not happen there:

- Placement beyond what the objects rebuilt in visits 138 to 201 do:
  an object put into a container whose parent is asked function 9, the
  seat flag, the door mark on a tile, the wall bits of data word 5 (four
  objects are asked function 6 in the trace, and here), and a group
  placed off the lot.
- The footprint. The trace's objects hold it, but it was not compared.
- Set To Next modes 8 and 9, and the Relationship primitives beyond the
  rows they write here.
- That a tile inside the border of the 64 by 64 arrays counts as on the
  lot. The prototype tests against the arrays, not the lot's size, and no
  object of this house stands between the two.

What did not match. Object 19's second frame has tree ID 0x8118 in the
trace and 0x118 here and in the house file, as before. Its visit comes
after visit 201. In the trace that frame is gone when the step returns:
the object's reset replaces its thread. Nothing here explains the bit.
Six type attribute words belong to the game that was running; with those
copied from the trace's start they are equal. The C runtime's random
stream is now equal without copying anything.

What changed in earlier work, and why:

- **The pinned digests of steps 3 to 12 changed** because of the two new
  fields and the indicators' 64 draws. The gated tests show each state is
  the one pinned under format 6: with the fires and the frame storage
  emptied and the stream put back to its old state, the encoding hashes
  to the old digest. The comparison of steps 10 to 12 with the trace was
  run again and is equal as before.
- **The people's constructors draw 7,084 numbers, not 7,074.** They draw
  until a value fits, and they now start 64 draws later in the stream.
  Nothing they draw is kept. The stream's state after them is the
  trace's. Before, it was only once the stream's state at the start of
  the load had been copied from the trace.
- **An earlier note said function 2 is the only thing asked of 191
  objects.** It is 198: seven more run a Load tree that only wakes the
  object.
- **Refresh modes 1 and 2 no longer fail.** They mark the object's room
  as not counted.

What is **unverified**:

- **Everything after visit 140 that is not an object field**: the global
  random stream, the globals, the arrays and object grids, the removal
  queue and the rooms waiting to be counted after visits 141 to 201. The
  trace has no state there. The object fields at the end of those visits
  are equal, which the random stream and the grids feed into.
- **The contents of the slots and of the frame storage.** The trace holds
  neither. The link words and the object grids, which mirror the slots,
  are compared, and so are the locals that come out of the frame
  storage.
- **That frame storage never written reads as zero**: beyond the frames
  a save holds, and in a new object.
- **A person that refers to an object that resets.** The original makes
  the person drop it. Here that fails. No person of this house refers to
  an object reset in visits 1 to 201.
- **Notify on an object that is inside a callback further up the chain
  of calls.** We look at its top frame.
- **Create New Object Instance for a GUID with no definition**, which
  takes the false branch here.
- **The lot before a load**, as before, but for one thing the trace now
  confirms: its two indicators' 64 draws.

What is not modeled: the redrawing, shadows and lighting an object asks
for as it moves; the interface being told an object left; mode 1 of
Refresh asking for a redraw; and the old-version conversions the original
applies to an object between restoring its position and running Load.

## 2026-11-13: The lot's rooms are worked out from its walls, and the load runs through the household selection

The original works out which room each tile is in from the lot's walls.
It does that after it has read a house's objects, and then gives every
object its room (load step 10), empties the removal queue (step 11) and
selects the household (step 12). `sims boot` now does all of that and
stops before step 13.

What was decided:

- **The rooms are state of the lot** (`World::rooms`), worked out by
  `World::rebuild_rooms`, which is the original's 0x479d9a. A room is a
  set of tiles that connect without crossing a wall. The map holds one
  room ID for each tile of each level. A tile that a diagonal wall cuts in
  two has the mark -5 there, and a record that names its two triangles and
  the room of each. The ground level's rooms are numbered from 0 and the
  upper level's from 1024, in the order a column-by-column scan finds
  them.
- **A room's tile list keeps its repeats.** A room lists its tiles in the
  order it reached them, and lists a tile again each time a neighbour
  reaches it while it waits. The original counts a room's size from that
  list, so a room of 27 tiles can list 46 and count as 46. We keep the
  list as it is.
- **The same function rebuilds the tile flags**, as in the original: which
  tiles are outside every enclosure, and which can be built on. The
  original has a branch for a flood that stands on a tile with a diagonal
  wall. A flood can only get there by starting there, at the lot's corner,
  and we refuse a diagonal wall on the border. So the branch is not
  ported.
- **Whether a room is outdoors is worked out with the rooms.** The
  original works it out the first time something asks and keeps the
  answer. Nothing between the two moments changes what it is worked out
  from, so the result is the same.
- **Step 10 is `Interpreter::assign_rooms`.** The first pass writes each
  object's room (`World::room_at`, 0x433bc5) into its data word 29. The
  second runs each object's function 7 tree as a callback. Scripts can now
  read three things about a room (scope 23): whether it is outdoors, its
  level, and its area. Any other room field fails and names itself.
- **The room count is `World::update_rooms`**, the part of the original's
  room update (0x466c3c) that needs only walls and floors: each room's
  half tiles, covered half tiles, walls and covered walls, and from them
  global 27, the lot's indoor area. The original runs its update after
  step 10 and before step 11, and so does `sims boot`.
- **Steps 11 and 12 are called.** `World::select_family` now also takes
  the family's six magic words and sets `World::magic_history`, which
  nothing set before. The family's ID is the ID of its record in the
  neighborhood file, not the number inside the record.
- **Refresh modes 1 and 2 still fail loudly.** They mark a room as
  changed. No tree in steps 10 to 12 reaches them, so nothing shows what
  else the original does there. Step 13 reaches mode 2.
- **The snapshot format is 6.** `World` has a new last field, the rooms. A
  lot with no rooms encodes them as four zero bytes. The gated tests show
  that each state pinned under format 5 is the same state without those
  bytes.
- **A lot with no wall array gets a bare lot's cells outside its size.**
  See "What was wrong" below.

How it was checked. The old prototype kept, privately, a trace of the
original loading the Goth house. It stops at the entry and the return of
step 10 (0x442659), at the entry and the return of step 11 (0x428679),
and when step 12's funds setter (0x46d8dc) returns. At each stop it holds
the objects, the globals, the random streams, the arrays inside the lot's
size, the 672 bytes of each room, and some pages of the game's memory. Our
state at each of the five moments was compared with it outside the
repository.

*Equal to the trace, the rooms (entry of step 10):*

- The room IDs: 0 to 4 and 1024 to 1029.
- How many tiles each of the 11 rooms lists.
- The listed tiles, in order, of the 8 rooms whose tile memory the trace
  holds: 1, 2, 4, 1025, 1026, 1028 and 1029 at this stop, and 1027 at the
  end of the load.
- The room of every tile of the upper level, in the 32 columns of 64 the
  trace holds at this stop.
- The number of split tiles, 7.
- The tile flags of both levels, every cell. The rebuild changes none: the
  flags it works out are the flags the house saved.
- For an empty lot of 52 tiles, which the dump tool makes by blanking the
  walls and floors: two rooms that list 5,307 tiles each, as the two rooms
  the trace has before the rebuild do.

*Equal to the trace, but at a later stop.* The room of every tile of both
levels, all 64 columns, equals the trace's memory at the end of the load.
The trace does not hold the ground level's map at the entry of step 10.
Its rooms have the same tile lists at both stops, so the maps are taken to
be the same.

*Equal to the trace, step 10 (return of step 10):*

- The trace asks function 7 of all 382 objects, in registration order.
- The 62 objects whose frame storage it moves are the 62 that have a
  function 7 tree here, in the same order. They are also the 62 whose last
  result changes.
- No data word changes in the trace, and the first pass changes none here.
  So the room worked out for each of the 382 objects is the room the
  house saved. 234 of them stand on the lot.
- All the fields compared after step 9 (see the entry below), for all 382
  objects, with the last result. The globals, both game random streams,
  the removal queue and the arrays.
- Whether a room is outdoors, for the 8 rooms the trace has worked it out
  for by then. Those are the rooms the 62 trees asked about.

*Equal to the trace, the room count (entry of step 11):*

- For all 11 rooms: the half tiles, the covered half tiles, the walls
  that can take a covering, the walls that have one, and whether the room
  is outdoors.
- Global 27, which goes from 0 to 204.
- Everything compared at the return of step 10.

*Equal to the trace, steps 11 and 12:* the same fields after each. The
removal queue goes from four IDs to none. In step 12 global 9 goes from 0
to 5, global 13 and the funds from 2,515 to 2,542; the selected family's
ID is 5, its magic history is marked as read and its six words are zero.
The trace's next stop, the entry of step 13, differs from that one in
nothing compared here.

The comparison is not empty. Between the stops the trace changes exactly
what we change: the rooms, 62 last results, global 27, the queue, and
globals 9 and 13 with the funds.

*Consistent with the old prototype's isolated comparisons only.* The
prototype ran the original's code alone, outside a running game, on
made-up inputs. These rest on that and on nothing in the trace:

- The room of an object on a split tile. No object of the Goth house
  stands on one. The prototype compared 32,780 positions.
- Which room each triangle of a split tile has. The trace's memory does
  not hold the records. The prototype compared 80 made-up layouts and
  six real floors.
- The edge of a pool as a room boundary. The Goth house has no pool.
- The parts of the flag rules this house does not reach. It does have
  sloped ground, fences, a fence on a diagonal, and an upper level with
  supported and unsupported tiles, and its flags are equal in every
  cell. The prototype compared 96 made-up cases.
- The refused cases: crossing diagonals, unmirrored walls, a diagonal on
  the border.

What was wrong, and is fixed. `World::read_arrays` keeps the lot's own
cells outside the lot's size, and a lot with no array had zeros there.
That was marked unverified. The trace's memory at the entry of step 10
holds most cells of every array, also outside the size, and all of them
at some stop for the walls. Every held cell equals ours but one kind: the
ground level's wall cells outside the 52 tiles. In the original each is
`00 0f 01 00 01 00` and eight zero bytes: no segments, flags 0x0f, both
styles 1. The held lot has that in every ground wall cell when the load
starts (the trace's memory at the entry of the resize), and the house
file saved the same bytes there. A lot here has no arrays before the
load, so `read_arrays` now gives the ground level's walls that cell
outside the size (`BARE_GROUND_WALL`). After the fix every held cell of
every array is equal. The digests of the arrays and of the states after
steps 6e and 9 changed with it. The gated tests show that zeroing those
cells again gives the digests pinned before.

The same memory settles two things the entry below left unverified. The
eight grid cells outside the lot's size that name objects standing
elsewhere are still there in the trace after step 9, as they are here. And
the cells outside the size of the other arrays are what we have.

What did not match. Object 19's second frame has tree ID 0x8118 in the
trace and 0x118 here, at all five stops, as before. Nothing in this work
explains it. The C runtime's random stream and six type attribute words
belong to the game that was running; with those copied from the trace's
start of the load they are equal at every stop.

What is **unverified**:

- **The listed tiles of rooms 0, 3 and 1024.** The trace holds how many
  there are, not which.
- **The split tile records**, beyond their number.
- **An object on a split tile**, as above.
- **Room fields 2 and 3 as a script reads them** (level and area). The
  trees of step 10 read only field 1. The area a script reads before the
  rooms are counted is 0 here, because the count is; no tree shows that.
- **Global 10 in step 12.** The house saved its own number there.
- **Selecting no family**, and a family record older than version 9,
  which has no magic words. `sims boot` refuses a house no family lives
  in.
- **The level argument of step 10.** The original can run both passes for
  one level only. We always run both levels.
- **Arrays 4, 6 and 7**, as before.

What is not modeled, and not compared:

- The rest of the original's room update: what each object adds to its
  room, the rooms' light and scores, the lot's outdoor score, the lighting
  of the tiles. In the trace this update changes no object, random stream
  or global but 27, so the state we compare is not affected yet.
- Which rooms have people in them. The first pass of step 10 tells a
  person's room that it is occupied.
- The rooms of the bare lot before the load. The original has two after
  the resize (step 3). `World::rooms` is empty until step 9a. No script
  that runs in between reads a room.
- Two booleans about the upper level, the roofs, and the redrawing.

What fails loudly: two diagonal walls crossing in one tile; a wall its
neighbour does not mirror; an enclosing diagonal wall on the border of
the arrays; more than 1,024 rooms on a level; a missing or misshapen
array; asking for a room before the rooms exist; a room field other than
1 to 3; Refresh modes 1 and 2 on an object that is in a room; an object
that removes itself while it is told the rooms changed; and booting a
house no family lives in.

What the trace shows about step 13, noted here because it sets the next
piece of work. The step asks function 2 of all 379 objects that are not
people. 184 objects have a changed type, and 158 of them have no Reset
tree, so they are taken off the lot and put back: the trace asks
functions 10 and 9 of each, and of every tile of its group. 148 of those
stand on the lot. So step 13 needs placement: taking an object off its
tile and putting it back, with the grid, the slots, containers and
groups. A trial run of the Load trees here also stopped at Notify Stack
Object out of Idle, Generic Sims Calls 11 and 43, and Refresh mode 2.
`docs/roadmap.md` has the numbers.

What would reopen this: a house with crossing diagonals or with an object
on a split tile; a capture that holds the split records or the tile lists
of the big rooms; a script that reads a room's area before the rooms are
counted; anything that reads a room's light or score, which needs the
rest of the room update.

## 2026-11-13: The controllers are reconciled among the saved objects, and the grid repair clears only what is inside the lot

Load step 8 makes the lot hold one of every lot-wide controller, now that
the saved objects are on it. Load step 9 clears cells of the ground
level's object grid that name an object which is not there. `sims boot`
runs both and stops before step 10.

What was decided:

- **Step 8 is the reconciliation of step 3, called on the full lot**
  (`Interpreter::reconcile_controllers`). Creating the missing controllers
  needed no new code: the factory, the primitives and the services the
  141 controllers reached on a bare lot were enough for the 137 created
  here.
- **Later duplicates are removed.** The original lists the objects whose
  definition has a nonzero global word once, in registration order. For
  each controller definition it keeps the first listed instance and
  removes each later one at once (`Interpreter::remove_now`), scanning the
  list again from the start after each removal. The function now returns
  what it created and what it removed (`Reconciled`), and so does
  `Interpreter::resize`.
- **An object can be deleted on a lot with people, if no person refers to
  it.** The original tells every person about a deleted object. A person
  that does not refer to it changes nothing. A person refers to an object
  when it is using it, or when its running, last or a queued action
  targets it or shows its icon. Deleting an object a person refers to
  fails with an error that names the person, before anything changes.
- **Step 9 is a step of `World`** (`World::repair_grid`), because it runs
  no script. It looks only at the ground level's grid, and only at cells
  inside the lot's size. A cell is cleared if its ID is negative, names no
  object, or names an object whose tile and level are not that cell on
  level 1. The tile is the position divided by 16, rounding down
  (`World::tile`).
- **"Restore placement links" is not a step.** The roadmap listed it with
  step 9, after the old prototype, which built its own link tables at
  this point. The original's function for steps 8 and 9 (0x43fc02) ends
  after the grid repair. The links are state the object read already
  filled: the grid, and each object's slots. Nothing is rebuilt.
- **Step 11 has its state change** (`World::discard_removals`). It is not
  called, because step 10 is not ported.
- **The snapshot format stays 5.** `World` has no new field.

How it was checked. The old prototype kept, privately, a trace of the
original loading the Goth house, with the game stopped at several moments.
One is the return of 0x43fc02, after steps 8 and 9. Our state after both
steps was compared with it outside the repository.

Equal for all 382 objects: the ID and GUID, the 72 data words, the temps,
the attributes and their number, the scheduler word, the last result,
each frame's node, stack object, arguments, locals and saved primitive
state, the relationships, the number of slots, the position, the tile and
level bytes the grid repair reads, and the mark of an object that asked to
be removed without resets. Equal for the three people: the 256 words of
person data. Also equal: the registration order, the tick, the lot size,
the 64 globals, both game random streams, the four IDs queued for removal
(42, 44, 278, 280), the scheduler cursor, the funds, the indicator
references, the one instance list, and inside the lot's 52 tiles the
terrain heights, both levels' floors, walls, object grid and flags, the
pool and the water.

The comparison is not empty. Between the stop after the object read and
this one, the trace gains 137 objects and two queued IDs and its global
random stream moves. Nothing else it holds changes: no saved object, no
global, no type attribute, no grid cell. The same is true here.

*Slots.* The trace also saved pages of the game's memory at each stop.
They hold the slots of 198 objects at this stop and of 149 saved objects
at the stop before, 216 objects in all. For each, the object ID in every
slot equals ours. Those objects hold 30 of the 32 links between objects
that our state has.

*The grid.* In the trace every occupied cell of both grids names an
object whose tile bytes are that cell, before and after step 9, so the
original cleared nothing. We clear nothing either. The saved grid does
name eight objects that are elsewhere, all in cells outside the lot's 52
tiles; scanning the whole 64 would clear them. That the scan stops at the
lot's size is from the original's code, which was read for this port, and
from the old prototype, which ran that code alone on made-up grids with a
size smaller than the array.

What did not match. Object 19's second frame still has tree ID 0x8118 in
the trace and 0x118 here, as after the object read. Nothing in steps 8 or
9 touches it. In the trace the frame is gone after step 13, when the
object's thread holds only its Main frame. It is the only frame with that
bit at any stop looked at. It stays unexplained.

The C runtime's random stream and six type attribute words belong to the
game that was running, as before. With those copied from the trace's
state at the start of the load, they are equal at this stop too. Step 8
draws nothing from the C runtime's stream.

What is **unverified**:

- **Removing a duplicate controller.** The Goth house has none, so no
  capture shows it. The selection follows the old prototype, which
  compared its loop with the original's 0x442b9f run alone on 97 made-up
  catalogs, with the factory and the removal replaced by stubs.
- **That a person who does not refer to a deleted object changes
  nothing.** From the old prototype, which compared its port of the
  person's notification with the original's code run alone. No capture
  holds a deletion on a lot with people. Which of a person's fields count
  as a reference follows that port too; where it tests the frames of the
  person's thread that use an object, we test the saved list of uses.
- **Clearing a grid cell.** No capture shows one being cleared. The rule
  is from the original's code and the old prototype's isolated runs of it.
- **Cells outside the lot's size.** The trace does not hold them. We keep
  what the house saved there.
- **The slots of the other 166 objects**, and every slot field but the
  object ID.
- **`World::discard_removals` in its place.** The trace shows the queue
  going from four IDs to none with no object changing. Our state at that
  moment was not compared, because the load does not reach it.

What is not modeled and fails loudly instead: removing a duplicate that
is on the lot, holds or sits in something, is multi-tile, or has a
callback for leaving the lot; removing one a person refers to; a
controller that a script removed while the controllers were being
reconciled, where the original would use freed memory; and a ground grid
that is missing or not 64 by 64 two-byte cells.

What is not modeled and not compared: the animation tester's ID, which
step 8 notes for the interface; the log the second pass of step 9 writes;
and two flag bytes the object registry sets around these steps.

What the trace shows about the next steps, noted here because it changes
the plan. Step 10 needs the lot's rooms, which the original builds
between step 9 and step 10 (the trace has 2 rooms before and 11 after).
Working rooms out from the walls is the next piece to port. And the two
IDs step 3 left in the removal queue, 42 and 44, name saved objects after
step 6e. Step 11 empties the queue before anything acts on it, so it must
not be skipped.

What would reopen this: a house that saved a controller twice; a house
whose grid names a missing object inside the lot's size; a capture of
either; or finding what sets bit 15 of a frame's tree ID.

## 2026-11-13: Saved objects are constructed and then read, and the load runs through the object read

Load step 6 reads the rest of the house onto the cleared lot. This entry
covers its five parts: the lot arrays, the floor and wall renumbering, the
type table, the second clear and the saved objects. `sims boot` now runs
all of them and stops before step 8.

What was decided:

- **Saved floors and walls are widened into the lot's arrays**
  (`World::read_arrays`). A house of version 53 to 62 saves a floor cell
  as one byte and a wall cell as 8 bytes. The lot holds 2 and 14. The
  original widens only the cells inside the lot's size. Here the cells
  outside it are zero when the lot had no array. Other house versions
  fail.
- **The renumbering is ported for an empty map only**
  (`World::renumber_surfaces`). A floor ID the map should have listed
  becomes 65535. A wall cell that would change fails. A map with entries
  fails.
- **Reading the objects is a step of the interpreter**
  (`Interpreter::read_objects` in `crates/sims-vm/src/objects.rs`). It
  needs the object definitions, which `sims-sim` does not hold.
  `World::read_objects` is gone. `World::from_house` stays as a way to look
  at a house file; it copies saved fields and constructs nothing.
- **A saved object is constructed first.** It gets the attributes its
  definition asks for (8 at least), a slot for its tile and one for each
  object it can hold, and its definition's dynamic sprite flags. The saved
  fields are read over that and never shrink it. A record with no
  scheduler word keeps the word registration gave.
- **Multi-tile links are state** (`World::next_parts`). Each tile joins its
  lead tile's list in the file's order.
- **A person's constructor is ported as far as it changes state we hold.**
  It reads the skeleton's file if the game has not read it yet, builds the
  skeleton, attaches the person to its neighborhood record, and makes
  default interests and personality if the record gives it none. Each of
  those draws from the C runtime's random stream. The saved person data
  replaces the defaults. Person data has 256 words; an older save's 80
  are followed by zeros.
- **Which skeleton files the game has read is state**
  (`World::skeleton_files`). It belongs to the running game, like the C
  runtime's stream.
- **`sims_content::People` finds what the constructor needs.** The
  skeleton's name is the first of the person type's body strings. The
  file is the one skeleton file that defines that name, among the
  archives in `GameData/Animation` and the indexed object directories. A
  name that two files define is an error.
- **The snapshot format is 5.** The two new fields are at the end of the
  encoding.

How it was checked. The old prototype kept, privately, a trace of the
original loading the Goth house, with the game stopped at several moments.
Three of them were compared with our state outside the repository. A
"capture" below is one of these stops.

*After the arrays and the renumbering* (the capture at the entry of
0x43fc02). The capture holds the cells inside the lot's 52 tiles of the
terrain heights, the floors, walls, object grid and flags of both levels,
the pool and the water. All equal ours. The capture taken before the
arrays are read differs from the house, so the comparison is not empty.
The house's two maps are empty, and none of its cells holds an ID the
renumbering covers, so the renumbering changes nothing here; the capture
agrees.

*After the object read* (the capture at the return of 0x449c5e). Equal for
all 245 objects: the ID and GUID, the 72 data words, the temps, the
attributes and their number, the scheduler word, the last result, each
frame's node, stack object, arguments, locals and saved primitive state,
the relationships, the number of slots, and the position. Equal for the
three people: the 256 words of person data. Also equal: the registration
order, the tick, the lot size, the 64 globals, both game random streams,
the two IDs queued for removal, the scheduler cursor, the funds, the
indicator references, the one instance list, and the arrays again.

One object shows why the constructor matters: its definition asks for 16
attributes and the save holds 11. The capture has 16.

*The random draws.* The C runtime's stream starts in a state that belongs
to the game that was running. With that state, and the six type attribute
words that also belong to it, copied from the capture at the start of the
load, our stream after the object read equals the capture's. So the
constructors of the three Goths draw as many numbers here as in the
original. The stream's state only depends on how many numbers were drawn,
so this does not show what each draw was for. In the capture the stream
does not move anywhere else between the start of the load and the end of
the object read.

*Multi-tile links.* The trace does not hold them. Another capture, of the
same house four ticks after its load, does: all 142 tiles name the same
next tile, lead tile, origin and offset as ours.

What did not match. One frame of object 19 has tree ID 0x8118 in the
capture and 0x118 in the save and here. Object 17 has the same type and
the same thread, and matches. The capture still shows the bit after
step 8. What sets it is not known. It is **unverified** whether it is
state.

What is **unverified**:

- **Arrays 4, 6 and 7, and every cell outside the lot's size.** The
  captures do not hold them. That a lot without an array has zeros outside
  its size is a guess.
- **The rewrite to 65535.** The Goth house has no cell it applies to. It
  follows the old prototype, which compared its port with the original's
  code run alone on 24 made-up lots.
- **What the draws are for.** The split into skeleton files, skeletons
  and defaults follows the old prototype, which compared each part with
  the original's code run alone. Only the total was compared with the
  running game.
- **That the game has read no skeleton file before its first load.** The
  total fits that for this capture. Another session might have read one
  already.
- **Which files the skeleton lookup searches.** The original uses a
  registry of avatar files. Here the name must be defined by exactly one
  file.
- **Footprints, the short after the position, the frame arena size, slot
  contents, sprite flags, motives and queues.** They are the saved values.
  The captures do not hold them, or were not decoded for them.
- **The other 110 changed flags and the rebuilt type table**, as before.

What is not modeled and fails loudly instead: a house version outside 53
to 62; a floor or wall map with entries; a wall cell the renumbering
would change; an object with graphics of its own; a person with no
neighborhood record, for which the original makes one; a skeleton that no
file or two files define; saved attributes beyond what the constructor
allocates; and reading objects onto a lot that holds objects.

What is not modeled and not compared: the lot's lighting, which the array
read resets; object flags; a person's animator, saved animations and
built-in slot positions; and the game's count of queued interactions.

How the earlier states were kept. The states pinned for the resize and
the first clear are the old encodings plus the two bytes of the new,
empty fields. The gated tests check that: without those bytes each
hashes to its format 4 digest.

What would reopen this: a house with custom floors or walls; a house
whose people have no neighborhood record; a capture that holds the cells
outside the lot's size; or finding what sets bit 15 of a frame's tree ID.

## 2026-11-13: Clearing the lot deletes objects one at a time, and the type table is read before the objects

Load step 5 removes every object the lot holds before the house's own
objects are read. Removing is not dropping: each object leaves the lot,
resets one last time, and is then unlinked. The reset runs the object's
Reset tree, a script, so the clear can change globals, draw random numbers
and queue removals. This entry also corrects the order of steps 6 and 7 in
`docs/roadmap.md` and says where the load now stops.

What was decided:

- **Deletion is ported for the simple case** (`crates/sims-vm/src/removal.rs`).
  `Interpreter::remove_now` takes an object off the lot and deletes it.
  `Interpreter::delete` releases the ownership of interface events, runs
  the last reset, and unlinks the object from the registration list and
  from its type's instance list. `Interpreter::reset` runs the Reset tree
  on top of the object's thread and then restarts the thread at Main; an
  object with no Reset tree only leaves the lot. `Interpreter::clear` is
  step 5: the two indicators first, then the head of the registration list
  until it is empty.
- **The removal queue keeps IDs whose objects are gone.** The original's
  queue holds bare IDs. The clear does not touch it, and the load erases
  it later (step 11). So after the clear `World::removals` still names 42
  and 44.
- **An instance list outlives its instances.** A type's list in
  `World::instances` stays, empty, when its last object is deleted. An
  object read or created later is appended to it.
- **`World` holds the indicators' IDs** (`World::indicators`, 0 for none).
  The clear needs to know which objects they are, and it sets both to 0.
- **The changed flag is state of the running game.**
  `World::changed_types` holds the GUIDs of the types whose scripts
  changed since a house that listed them was saved. The original keeps
  the flag on each definition. Reading a type table sets or clears it for
  the types the table lists and leaves the others alone.
  `Interpreter::read_types` does that and rebuilds `World::types`.
- **Steps 6 and 7 run in another order than the roadmap said.** After the
  clear the original reads the lot arrays, renumbers the saved floors and
  walls, reads the type table and marks the changed types in the same
  pass, clears the lot a second time, and only then reads the objects. The
  roadmap had the changed types after the objects. The step table now
  lists 6a to 6e.
- **`World::from_house` is split.** `World::read_arrays` and
  `World::read_objects` read onto the lot the load holds and keep what the
  earlier steps left. `from_house` stays as a way to look at a house file.
- **`sims boot` stops after the lot arrays.** The next thing the original
  does, renumbering floors and walls, is not ported.
- **The snapshot format is 4.** The two new fields are at the end of the
  encoding.

How it was checked. The old prototype's harness captured the original at
the moment the clear returns, loading the Goth house. Our state was
compared with that capture outside the repository. Equal: no objects, an
empty registration list, the tick, the lot size, the 64 globals, both game
random streams, the queued removals (42 and 44), the scheduler cursor, the
funds, both indicator references (0), the one instance list (empty), and
the type attribute records' GUIDs, order and lengths. Eighteen controllers
ran a Reset tree on the way. The test
`the_goth_house_clear_removes_every_object` pins a digest of our state.

The state when the resize returns was not compared again. Its encoding is
the old one plus the three bytes of the new fields, and without those
bytes it hashes to the digest the earlier comparison pinned.

For the type table, a capture taken after the original had read it shows
the three Goths' own types: an Init signature of 20 for each, the changed
flag set, and a Main signature never worked out. We get 20 and the flag
for all three. That also confirms the rule that Main is only looked at
when Init matches.

What did not match, and why. The same things as when the resize returns,
and for the same reason: the C runtime's random stream and six type
attribute words belong to the game that was running. With those copied
from the capture into the lot before the resize, they match after the
clear too, so no Reset tree changes either one here.

What is **unverified**:

- **The lot arrays** (`World::read_arrays`). No capture covers the
  arrays after they are read. Which arrays are read, and in what order,
  comes from the old prototype's reading of the code. A missing array is
  an error here; the original stops reading arrays and carries on.
- **The other 110 changed flags and the rebuilt type table.** Ported from
  the old prototype, which compared its version with the original's code
  run alone on made-up records.
- **`World::read_objects`.** No capture covers the object read. It copies
  saved fields and does not construct anything, so it is not a load step
  yet and `sims boot` does not call it.
- **Deletion paths the 143 objects do not take.** A Reset entry whose
  tree is missing, a Reset tree that removes or creates objects, and an
  object that owns interface events are ported from the old prototype and
  covered by synthetic tests only.
- **The reset outside a deletion.** `Interpreter::reset` with a Reset
  tree is the same code path, but nothing calls it that way yet.
- **The leftover type table.** After the clear `World::types` still lists
  the controllers' types. That table is our bookkeeping, and the type
  table read replaces it.

What is not modeled and fails loudly instead: removing an object that is
on the lot, in a container, holding something, multi-tile, a person, or
has a callback for leaving the lot; deleting anything while a person is on
the lot, because people must drop their references to it; resetting an
object with no Reset tree outside a deletion, which rebuilds and replaces
it; reading a type table onto a lot that holds objects; a lot array that
is missing or not 64 by 64; and a house with a GUID translation table.

What is not modeled and not compared: the interface being switched off
during the clear, the objects' graphics, the pools of spare indicators,
the redraw of the room a removed light was in, the deletion mark on the
object (only a person's reset reads it), type ID 0 in the rebuilt table,
and type tables older than version 2.

What would reopen this: a capture taken after the arrays or the objects
are read; a lot whose objects are on tiles when it is cleared; or a house
with custom floors or walls, which makes the renumbering matter.

## 2026-11-13: A load starts from the lot the game holds, and resizing it creates the controllers

The original keeps one lot for as long as it runs and loads each house into
it. Before it reads a house's objects it reads the house's simulator state,
resizes the lot to the house's size, and that resize creates every lot-wide
controller the lot lacks. It then removes every object and only then reads
the saved ones. So 141 controllers run their Init and first turn on a
nearly empty lot, draw random numbers and write globals, and are thrown
away. The load sequence in `docs/roadmap.md` now starts with these steps.
This replaces the ten-step sequence of "The lot load follows a written
sequence" below, which read the whole house first, and that entry's mode 0.

What was decided:

- **`World` is the lot the game holds, not a house.** `World::empty` is the
  lot before any load, `World::read_simulator` reads a house's simulator
  state into it, and `Interpreter::resize` shrinks it. `World::from_house`
  still reads a whole file at once; it is no longer the load path and will
  be split when the saved objects are read onto the cleared lot.
- **The object factory is ported** (`Interpreter::create`): lowest unused
  ID, head of the registration list, the constructor's storage, the date
  stamp, the reset, Main installed, Init run synchronously above it, one
  first turn, then the scheduler flags and the price.
- **Sleep yields.** An object that is not a person and not locked out
  hands its count to the scheduler, which is the low half of the registry
  word. `Interpreter::turn` runs a thread until it yields or returns.
  Resolving data word 25 wakes an object, as in the original.
- **New state.** `World` gains the C runtime's random stream, the lot's
  size, the objects waiting for removal, the type attributes, the per-type
  instance lists, the owner of interface events and the selected family's
  magic history. `Object` gains its last result and a removal flag. The
  snapshot format is 3.
- **Frames of new objects need a type ID.** A frame names its code by the
  lot's type table, which the original only builds when it saves. A type
  the table lacks takes the lowest unused ID. This is our bookkeeping; no
  original state corresponds to it.
- **The whole catalog is linked.** The resize walks every definition, so
  `sims boot` links them all. `Program` also holds each file's interaction
  tables and slot counts, the career list from `work.iff`, and the
  neighborhood's lot zoning.
- **The context's mode is 1.** `apply_context` no longer clears the family
  in global 9; `World::clear_family` does, after the resize, because the
  controllers must still see the saved family.

How it was checked. The old prototype's harness captured the original as it
loaded the Goth house, at the moment the resize returns. Our state was
compared with that capture outside the repository. All 141 controllers
match: GUID, the 72 data words, temps, attributes, last result, every
frame, the registry word and the position. So do the 64 globals, both game
random streams, the tick, the lot size, the registration order, the two
controllers waiting for removal, the scheduler cursor, the funds, the
instance lists, and the type attribute records' GUIDs, order and lengths.
The test `the_goth_house_resize_creates_the_controllers` pins a digest of
our state and runs when `SIMS_INSTALL` is set. The capture is private.

The mode is 1 on the capture's evidence: at the start of the resize, before
any script has run, global 15 is 1 while the house saved 0.

What did not match, and why. Three things in the capture belong to the game
that was running, not to the house or the install:

- The two indicator objects. Ours are created on an empty lot whose clock
  is zero, and have just gone to sleep. The capture's carry another
  creation date and had been asleep for a while.
- The C runtime's random stream, which no file holds.
- Six type attribute words, one for each help system, which are set in
  the capture and clear in the neighborhood file.

With those copied from the capture's state at the start of the resize, the
two indicators match as well, which makes all 143 objects. The controllers
match either way.

What is **unverified**:

- **The starting lot.** The creation date and time the capture's
  indicators carry is exactly the clock saved in `Houses/House00.iff`, the
  empty lot every neighborhood ships with. So the original most likely
  loads House00 when it starts, and the indicators come from that load. We do not model it: `sims boot`
  starts from `World::empty`. This cannot change what a load leaves behind
  unless a controller reads the indicators' state, and none did here.
- **The six type attribute words.** Something in the running game set
  them before the load. We do not know what. The 141 controllers end in
  the same state with either value, but later ticks may not.
- **The C runtime stream's starting state.** It is 1, the runtime's own
  default. Nothing in this step draws from it.
- **Fields the capture does not hold.** A new object's footprint, the short
  after it, its frame arena size, its relationship flag and its slots are
  set to what the original's saves hold for such an object.
- **Paths the 141 controllers do not take.** Sleep, waking, Remove Object
  Instance, Generic Sims Call 28 and 43, event ownership, Set To Next
  modes 4 and 10 and the type attribute scopes are ported from the old
  prototype. The capture confirms the paths these controllers take through
  them, not the others. The original keeps an object with a headline
  showing out of the scheduler's hands while it sleeps; headlines are not
  modeled, so that test is missing.
- **The turn budget.** Init and the first turn may each run 100,000
  instructions, the old prototype's guard. The original's limit is not
  known.
- **Which zoning file the original reads.** We read the neighborhood's
  `LotZoning.iff`; the old prototype read the install's
  `MasterLotZoning.iff`. Both give the same table for the shipped
  neighborhoods.
- **Global 25 and free will in the context.** The capture agrees with
  them, but the house saved the same values, so it proves nothing.

What is not modeled and fails loudly instead: growing a lot; resizing a lot
that has arrays or objects standing on it; a second instance of a
controller, which needs deletion; removing an object that is on the lot,
holds or sits in something, is a person, or has a callback for leaving the
lot; the factories for people, cursors and multi-tile objects; Generic Sims
Call routines other than 28 and 43, and 43 once a family's magic history is
held; a type that wants its attributes cleared for each lot.

What is not modeled and not compared: the lot's rooms and their tile
counts, the interface's action counter, and the interface's lists of
indicators. The resize only writes the indoor area of a bare lot, which is
0, to global 27.

What would reopen this: a capture of the original at startup, which would
settle the starting lot and the type attribute words; a house whose
controllers take the unverified paths; or a load into a lot that is not
bare.

## 2026-11-13: The catalog follows the original's scan order and file tables

Objects register in the order the original scans for them (0x43c609):
`GameData/Objects`, `GameData/UserObjects`, the neighborhood's
`Characters`, `Downloads`, the packs in bit order up to Superstar, the
shared directories, and Makin' Magic last. `Index::add_characters` puts
the people in that place. This replaces "characters register after the
install" below.

Within a directory the original does not use the file system's order. It
keeps each directory's entries in a hash table keyed by lower-case name and
walks the table (0x5ba9bd). `sims-content`'s `file_table` reproduces the
hash, the bucket counts and the growth rule, ported from the old prototype.
An archive's members are entries of the directory the archive is in, and a
member whose name holds a path goes into that subdirectory, whether or not
it exists on disk. The first entry of a name wins, except that a file on
disk replaces an archive member. This replaces "directory order is NTFS
name order" below.

It is checked against the original. When the Complete Collection loads a
lot, it creates the lot-wide controllers the lot lacks by walking its
catalog. A capture from the old prototype's harness holds the 141 objects
that made, in order. Picking the same definitions from our catalog gives
the same 141 GUIDs in the same order. The test
`the_catalog_lists_controllers_in_the_originals_order` pins a digest of
that order and runs when `SIMS_INSTALL` is set. The capture itself is
private.

What that leaves **unverified**:

- The order of two names that share a bucket, and which of two archives
  supplies a member both hold. Both depend on the order Windows lists a
  directory in, for which `Index` uses NTFS name order. The 141 controllers
  show no such case.
- Where the people fall. No controller is a person, so their place rests on
  the old prototype's reading of the scan.
- Houses saved by the original list their object types in catalog order,
  and those lists disagree with the capture in one way: they show larger
  tables for some directories (389 buckets for the base game's objects,
  not 193). The studio's machines presumably had more files in those
  directories. The capture is from the install we support, so it wins.

## 2026-11-13: The lot load follows a written sequence and stops at the first gap

The load sequence in `docs/roadmap.md` lists the ten steps the original runs between
reading a house file and the first tick, with what `sims boot` does for
each. `sims boot` runs them in order and stops at the first step that is
not implemented, so the hash it prints is always of a state the original
passes through. The alternative, running whatever callbacks already work,
would give a state the original is never in, and its hash would mean
nothing to the comparison harness.

The first step added is the globals the running game supplies
(`World::apply_context`). The values for a Complete Collection install
(mode 0, global 25 zero, free will on, expansion mask 0xff, performance
class 2) are the old prototype's headless profile and are **unverified**.
The performance class comes from the machine's memory and CPU speed, so
the comparison harness must read all five from the running original.

Writing the sequence down showed that the catalog order was wrong for load
step 4, where new objects get IDs in catalog order. The entry above fixes
it.

## 2026-11-13: Init primitives stop at state the lot does not hold

Test Object Type, Set To Next, Refresh, Play Sound Event and the stack
object's definition words (scope 21) are ported from the old prototype.
`Program` now keeps every OBJD as the 108 words the original holds after
registration: versions 136 and 137 become 138, and every record is cut or
zero-padded to 216 bytes (0x431759), so every scope 21 index under 108
reads. Definitions are listed in the file's native resource order, as the
catalog registers them; the prototype used physical file order.

Refresh mode 0 only redraws, so it does nothing here. Modes 1 and 2 mark
the object's room dirty, which makes the next room update rebuild the
room's score and lighting. Rooms are not modeled, so those modes fail
loudly unless the object is in no room (data word 29 is -5). This is now
the most common init failure on the Goth house. Play Sound Event only
queues audio, so it takes the true branch and changes nothing.

Set To Next walks registration order (modes 0, 2 and 6), people (1),
multi-tile groups (3) and families (12). Mode 4 fails loudly: the
original keeps a per-GUID list of instances for the whole process. The
list is filled in ID order on first use and appended to as objects
register, even across lot loads, and `World` has no such list. Modes 5
and 7 to 11, and unknown modes, also fail loudly. Three choices are
**unverified**:

- The person vector is the people in registration order. That is how
  loading builds it; Sims joining or leaving may change it.
- A multi-tile group's chain is its head, then its tiles in registration
  order, which is how loading links them (0x423223). `World` keeps only
  each tile's head, so splitting or joining groups at run time would need
  the links themselves.
- Test Object Type on a tile whose group has no catalog master, and Set
  To Next mode 6 in a tree with no parameters, fail loudly. What the
  original does there is not known.

The three load callbacks that fail with "tree 0x100b is missing" are not a
linking bug. `FlowersOutdoor.iff` OBJD 16808 has no OBJf, so its callbacks
come from the OBJD itself, and its load field names 0x100b. The file has
no such tree and no semi-global file. The prototype found that the
original skips a callback whose tree is absent (0x4749b5). Whatever runs
callbacks will have to do the same.

## 2026-11-13: The interpreter runs on the saved frames

An object's thread is its stack of saved frames in `World`, so the
interpreter keeps no state of its own and every snapshot holds a resumable
stack. Frames name their code owner by objt type ID, so `World` now keeps
the lot's type table (`types`, type ID to GUID; snapshot format 2).
Expression, Random Number and the operand scopes are ported from the old
prototype with its native error codes. Scopes over state `World` lacks
(tree advertisements, definition words, rooms, neighbors, jobs,
neighborhood data, type attributes) and every other primitive fail loudly.
Four semantics are **unverified**:

- New frames start with zeroed locals. The original reuses a frame arena,
  so locals inherit words left by earlier frames; saves carry no arena.
- Random Number draws from a stream the caller names. The original swaps
  the lot stream in only around the lot update (0x46d553).
- Motive reads round half to even (the x87 default); the prototype rounded
  half away from zero.
- Motive arithmetic rounds once from the x87's 64-bit significand to 32
  bits. The FPU precision the original runs it at is not known.

## 2026-11-13: Linking is checked against saved behavior signatures

Each object type in a save records two signatures: for its init and main
trees, the root's revision word shifted left 16 plus the revisions of every
tree reachable from the root (0x4026ed, recovered by the old prototype).
`sims boot` recomputes them from the linked program. Across 296 shipped
houses, 37,655 of 47,359 saved types match. In the best houses nearly all
match, and the misses differ by a revision or two, as when a later pack
revised a tree after the house was saved. A linking mistake would give
unrelated numbers.

On the Goth house only 36 of 113 types match: it was saved by the base
game, and the expansions revised most of its objects' code. The original
resets a saved object's script state when its signatures differ
(0x43a516), so booting this house exercises that reset for 77 types. The
interpreter has to implement it before the first tick.

## 2026-11-13: The Complete Collection ISO is the supported install

Decided by the maintainer. The vision named the Legacy Collection as the
primary install; it now names The Sims Complete Collection, installed from
its ISO, and leaves other editions out of scope. That is the install the
emulator image already holds and every check so far ran against, so no
code changes. It replaces the entry below on recognizing only the CD
layout: that is no longer a gap.

## 2026-11-13: The state hash is SHA-256 of the postcard encoding

`World` is encoded with postcard: fields in declaration order, integers as
little-endian varints, floats as raw bits (NaN payloads and negative zero
kept), and maps in key order. Nothing in it depends on the platform, and a
pinned-hash test runs on both CI platforms. Snapshots store the same bytes
behind a 4-byte format number, in a REDB table keyed by name. Changing the
encoding means bumping `snapshot::FORMAT` and the pinned hash. Comparison
with the original goes field by field, not through this hash: the hash only
proves that our own runs repeat.

`World` keeps every field the readers decode for objects, the simulator
and the lot arrays. It leaves out bytes nobody understands yet (format
tails, raw ObjM records), the HOUS view and model, and the wall and floor
chunks. They join when the interpreter or the comparison needs them.

## 2026-11-13: The target house is the Goth house

The boot goal needs one house. It is `House05.iff` in the base game's
shipped neighborhood: the Goth family (Mortimer, Bella and Cassandra), 245
saved objects, SIMI and ObjM version 0x3e. The installed copy is byte for
byte the shipped one, so the emulator can load it without copying files in.
The Newbie house (`House07.iff`) is smaller but was played and saved by the
old harness, so the installed copy differs from the shipped one. Reopen if
the Goth house needs something the harness cannot capture.

## 2026-11-13: Placeholder objects carry no registry word

Every ObjM record ended with a field-encoded registry word, until the
expansion houses: the `*Placeholder` object (GUID 0x3C08965F) never has one.
Across 139,000 ordinary records in the installed and shipped houses, 24
to 31 bits remain after the dynamic sprites. Across 27,000 placeholder
records, 0 to 7 remain, so no field fits. The reader now reads the word only
when at least a byte of the record remains, and stores None otherwise.
Why the original writes no word is unknown; it may skip objects missing
from the scheduler's registry. Unverified until the harness compares a
placeholder.

## 2026-11-13: Neighborhood characters register after the install

Superseded by "The catalog follows the original's scan order and file
tables" above: characters register after the user objects.

A house's Sims are defined by files in the neighborhood's `Characters`
directory. `Index::add_directory` appends that directory as one more mount,
after Downloads, so the catalog registers characters last. Where the
original registers them is unverified. A family with house number 0 has no
house: in the shipped and installed neighborhoods every such family lacks
flag bit 0, and every family with it has a nonzero house.

## 2026-11-13: Some installed houses do not load yet, on purpose

`sims houses` loads 660 of the 718 house files in the install's eight
neighborhoods and the shipped one. The 58 failures:

- 18 are `House00` and `House11` in each neighborhood: base-game templates
  with ObjM versions 0x33 and 0x38, whose record layout is not known yet.
- 40 are houses 56, 63, 69, 76 and 77, which hold Sims whose character files
  are not in the neighborhood: Superstar's community templates, and houses
  the old harness saved. The original presumably repairs or drops such residents,
  which we have not reverse engineered.

Neither blocks the target house. Reopen when a goal needs those houses.

## 2026-11-13: Later GUID registrations win; semi-globals resolve to the first match

The catalog registers object definitions in load order, and a GUID resolves
to its last registration. The old prototype recovered that rule from the
original's lookup. Multi-tile masters are the exception: the first
registered master wins, and that is for the interpreter to apply. A GLOB
link resolves to the first file of that name in load order. That part is
unverified: the Complete Collection has no duplicate semi-global names, so
nothing tells the two rules apart yet.

## 2026-11-13: Directory order is NTFS name order (unverified)

Superseded by "The catalog follows the original's scan order and file
tables" above: NTFS name order now decides only the order of names that
share a bucket.

The original lists each mounted directory with FindFirstFile, and that order
decides the object catalog order. On Windows/NTFS it is name order with case
folded. `Index::build` uses that order and searches a subdirectory where its
name falls among the files. Neither detail has been confirmed against the
original's walker yet. The old harness ran the game under Wine on ext4,
whose directory order can differ from NTFS. The comparison harness must
either record the order the original saw and feed it to us, or put the
install on a file system that lists in NTFS order.

## 2026-11-13: Expansions come from the pack directories

The original reads the installed packs from the registry (`EPxInstalled`,
`SIMS_GAME_EDITION`). We have no registry, so a pack counts as installed
when its directory holds a FAR archive. On the Complete Collection install
this gives edition 0xFF, the same as its registry.

## 2026-11-13: Only the CD layout is recognized for now

The CD layout is checked against a real Complete Collection install. The
Legacy Collection's layout is not known here, and a guessed table would not
be tested by anything. It is added when someone can list a real Legacy
Collection install.

## 2026-11-13: Readers stay strict on unreferenced bad chunks

21 TTAB chunks in the original install are version 3 or zero bytes long.
Nothing references them. The reader keeps rejecting them, and loaders
decode only the chunks they use. Reopen if a referenced chunk ever fails.

## 2026-11-13: SIMI versions 0x33 and 0x38 use the later layout (unverified)

Only the template houses carry these versions. Their fields up to the budget
read plausibly with the 0x3e layout, and 0x33's unread tail is 24 bytes
shorter. Check the fields against the original when the harness loads one
of these houses.

## 2026-11-13: SPR# versions 503 and 505 share the 502/504 frame encoding

All 14 such sets in the install decode exactly with it. What the version
number itself selects is unknown.
