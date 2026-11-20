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
  selected Sim's place among the household's members, is not -1.
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
- The panel's household notification (0x51ce79, for flags 0x20080000)
  reads the selection (0x4cf9a4). With none (-1), it opens kind 0
  (0x51d06e to 0x51d078). With one while kind 0 is open, it opens kind 1,
  the panel the constructor opens (0x51a0bc), which this port has as the
  needs (0x51d082 to 0x51d095).
- A panel change closes the help balloon (0x51d383 to 0x51d397), which
  plays `UI_whoosh`.

## What changes here

- `Controls::panel` is an `Option<Panel>`; `None` is kind 0.
- A gadget button pressed with nobody selected asks for
  `Command::Refused { error: false }`. The House button asks for
  `Command::Refused { error: true }` with nobody selected or on a lot that
  is not residential (`Snapshot::residential`). The buttons are not drawn
  disabled: the original's take the press.
- `Command::Refused` is presentation only. The game plays `UI_error` for
  the interface on `error`.
- `Controls::follow` takes whether a Sim is selected: with none, no panel
  is open; with one and no panel, the needs open. The game and the viewer
  call it before each layout. It closes the help balloon as a panel change
  does, and the game plays `UI_whoosh` then.
- `sims_vm::Lot::residential` gives the hosts the idle loop's check.
- This corrects `20261115-house-panel-size-yard-and-facts.md`, which says
  the house panel shows with or without a selected Sim. Its draw
  (0x52e49e) needs none, but with nobody selected no panel is open, so it
  does not show.

## Not ported, and unverified

- The original reacts to the household notification. Here the panel
  follows the selection at each frame. Whether a selection change always
  sends that notification is **unverified**.
- Another notification (0x51ce1d) turns kinds 6 and 7 back into kind 1.
  Not ported: which panels those kinds are is not traced.
- The button's off state (0x5e480a) is not modelled. A button here shows
  selected only while its panel is open.
