# Switching the selected Sim refreshes the deselected Sim's animator; a second press centres the view

*Decided 2026-11-15.*

A press on another Sim's portrait now switches the selection in strict
mode too. Deselecting a Sim refreshes its animator (0x58da12 with 0,
from 0x446921). That refresh failed in strict mode (`Refresh::Fail`) and
was left out in play mode (`Refresh::Skip`, `Lot::select_play`; the
entry "Deselecting a Sim is ported but for its animator's refresh"
below). Both are gone: the refresh is ported (`Interpreter::deselect`).

- **The refresh** (read from the code): with 0, the gate on animator
  byte +0x109 (whether the house view drew the person) is skipped, so
  it always runs. It is the same path a take-off takes when the view
  drew the person: the clock read again (virtual function 0x34, the
  lot's tick count), the skin forced up to date at it (0x5992b3 with 1),
  the animator placed (virtual function 0x60), and the head's step,
  measurement and restoration (0x590768, 0x5904d2, 0x590b27). Then the
  head slot's holder is redrawn (0x58dae5 to 0x58dc55, the third
  container entry). The marker has just left that slot (0x4468fd comes
  first), so it is usually empty. A person or a group tile there fails
  (`head_slot_redraw`). It reuses `Interpreter::refresh_animator`.
- **The clock between ticks**: the selection runs between lot updates,
  and the animator's clock is the lot's tick count times its per-tick
  step (0x585483). Our `World::ticks` is raised after the lot's pass,
  and the recordings compare it after every update. So between ticks
  it is the count the original reads.
- **The portrait press** (0x4cf9c4): on another Sim's portrait it
  selects through 0x4e989b. On a residential lot (0x40745f), 0x4e989b
  selects the person when person word 32 is 0, else nobody. Global
  0x6e4f64 overrides this; it is set at 0x4f961d and not read further.
  Then 0x445449 runs. On the selected Sim's own portrait it centres the
  view on the Sim instead (0x434ced with 1). Both play `UI_click`
  (0x4cfa51).
- **The centring** (0x434ced): nothing unless object data word 34 is 0
  and word 29 is not negative. A person on the floor shown is centred
  above its feet by trunc(2√6 · 2.0 · 2^(zoom+1)) / 2: 19, 39 and 78
  pixels from far to near. The zoom is the house view's virtual
  function 0x14, which returns 0x6f00d8 (vtable 0x668744). The person's
  screen point is the sound code's (0x47bc1f about 1023, 0x4a9dc7,
  `sound::object_point`). The original slides the view there
  (0x4c9a27); the window jumps. It also clamps to the scroll limits
  (0x4a9fa9), which the window does not have. Those limits keep the
  original's origin small enough for the point's 16-bit cut; the
  window's origin can go further, so it works the point out from the
  lot's corner and adds the origin after. Anything else, or a person
  on another floor, shows its floor (0x4a80c7) and is centred without
  the rise unless off the lot (0x4a7af0).
- **Verified**: the Goth house in strict mode selects another Goth, back
  again, and runs 50 more ticks with no error
  (`the_player_selects_another_goth`). The deselected Sim's animator
  clock is the tick count's. Through the window
  (`a_press_on_a_portrait_selects_that_goth_who_then_takes_the_order`),
  a press on another Goth's portrait at tick 50 selects her (person 18)
  and plays `UI_click`. A second press centres the view on her and
  changes nothing on the lot. Have A Snack from the fridge's pie goes to
  her, not to Mortimer, and she takes it 52 ticks later. Strict mode then
  runs to tick 454. At tick 455 Mortimer, left to himself, routes to a
  door one of his routes goes through (0x45b176). That is not ported, and
  it is track 4's routing.
- **Unverified**: no recording yet compares the lot after a switch.
  Track 2 is asked for one, with the switch, a second press, and an
  order to the second Sim. Also unverified: the people panel's call
  0x51ea7a on a second press, when global 0x6f6bac (cast) is not the
  Sim. It closes the panel's window at +0x31c and calls 0x586db2 with 0,
  presumably ending a camera follow. It is left out. Also unverified: a
  household member who is not on the lot (0x4e9652), and the lot's
  bounds used by 0x4a7af0 (taken here as its 64 by 64 tiles).
