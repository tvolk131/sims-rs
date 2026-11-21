# The live panel's buttons need a selected Sim, and House a residential lot

*Decided 2026-11-20.*

The live panel's seven gadget buttons (Needs, Interests, Relationships,
Personality, Job, Gifts, House) opened their panels on any press. The
original's press handler checks first. This port follows it.

## The original

- The press handler (0x51bbe6 to 0x51be75) tests the pressed button
  against the seven button fields (+0x15c, +0x16c, +0x160, +0x164, +0x168,
  +0x170, +0x174). Each opens its panel kind (1, 2, 5, 3, 4, 6, 7) through
  0x51d375, but only when a Sim is selected: the panel's +0xe8, the
  selected Sim's place among the household's members, is not -1. The
  check reads +0xe8 at the press (0x51bdf2), and House reads the zoning
  then too (0x51be03).
- The constructor gives the seven fields their art from the table at
  0x66c5c4 (from 0x518ab2): +0x15c 4500, +0x160 4504, +0x164 4503,
  +0x168 4502, +0x16c 4501, +0x170 4505, +0x174 4507. So kind 1 is the
  needs, 2 the personality, 3 the job, 4 the house, 5 the relationships,
  6 the interests and 7 the gifts.
- Without a selected Sim, the press sets the button off (0x5e480a(0) at
  0x51be6e) and does nothing else: no panel, no sound.
- The House button (+0x168, kind 4) also needs a residential lot: 0x40745f,
  the zoning of global 10's lot through 0x42b9e3 (lot 0 counts as
  residential), is 1. A House press without a selected Sim or on a lot
  that is not residential goes to 0x51be1a. That sets the button off and
  plays `UI_error` (0x6c5e90) for the interface (object -4) through
  0x57a54c.
- 0x51d375 itself turns kind 4 into 0 on a lot that is not residential
  (0x51d3f9). Its only kind-4 caller is the House press, which has
  checked already.
- Kind 0 is no panel: 0x51d375 hides every subpanel and shows none
  (0x51d4bf to 0x51d736).
- The panel's household notification (0x51ce79) reads the selection
  (0x4cf9a4). Flags 0x20080000 gate only the portraits' rebuild
  (0x51ce99); every notification goes on to the selection check. With no
  selection (-1), it opens kind 0 (0x51d06e to 0x51d078). With one while
  kind 0 is open, it opens kind 1, the needs, which the constructor opens
  too (0x51a0bc; 0x51d082 to 0x51d095).
- The selection's refresh turns the interests and the gifts (kinds 6 and
  7) into the needs when the selected object's +0x60e is above 1
  (0x51ce13 to 0x51ce31). The review read that as a pet; that is
  **unverified**.
- A panel change closes the help balloon (0x51d383 to 0x51d397, through
  0x50fb7f), which plays `UI_whoosh`.

## What changes here

- `Controls::panel` is an `Option<Panel>`; `None` is kind 0.
- The gadget buttons always ask for their panel, and are not drawn
  disabled: the original's take the press. The host checks the request
  at the press, from the lot as it is then (`hud::check_press`):
  - a button pressed with nobody selected becomes
    `Command::Refused { error: false }`;
  - the House button with nobody selected, or on a lot that is not
    residential, becomes `Command::Refused { error: true }`.
- `Command::Refused` is presentation only. The game plays `UI_error` for
  the interface on `error`.
- A selected person who is not one of the household's counts as nobody
  (`Snapshot::selected_resident`), as the panel's other parts already
  treat one.
- `Controls::follow` takes whether a Sim is selected: with none, no panel
  is open; with one and no panel, the needs open. The game and the viewer
  call it before each layout. It closes the help balloon as a panel change
  does, and the game plays `UI_whoosh` then.
- `sims_vm::Lot::residential` gives the hosts the idle loop's check. The
  game passes it with the lot's world to each press. The viewer does not
  read the lot's zoning, so there the lot counts as residential, and a
  refusal is silent: the viewer plays no sounds.
- This corrects `20261115-house-panel-size-yard-and-facts.md`, which says
  the house panel shows with or without a selected Sim. Its draw
  (0x52e49e) needs none, but with nobody selected no panel is open, so it
  does not show.

## Not ported, and unverified

- The original reacts to the household notification. Here the panel
  follows the selection at each frame. Whether a selection change always
  sends that notification is **unverified**.
- The refresh's turn of the interests and the gifts into the needs for a
  selected object whose +0x60e is above 1 (0x51ce13). Not ported: what
  +0x60e holds is not traced.
- The button's off state (0x5e480a) is not modelled. A button here shows
  selected only while its panel is open.
