# 2026-11-24: The game loop carries out the trips scripts ask for

## Context

#426 ported the phone's Go Downtown (`0x2075`) in the VM. The plugin pays
the fare with Alter Budget, asks for a lot, and runs Generic Sims Call 17.
Routine 17 (`0x45a22d`) writes the traveler's saved state and arrival clock
as tokens, then asks for the trip. That request is `World::trip_request`.
`sims_vm::travel::depart_from_request` carries it out.

The shell did not use either. When a destination lot picker was answered
(`Game::take_pending_travel_lot`), it planned a trip of its own and left
through `depart_to_destination`. That function writes the tokens and charges
the fare. The Go Downtown script asks for its trip a tick after the pick,
so the shell had already left by then. The fare was paid twice, 50 by the
script and 50 by the shell. The shell also guessed the party: for Magic
Town it sent the whole household, while the Magic Hole's script asks for
the jumper alone (routine 17's mode 0).

In `Sims.exe` 1.91, `0x4fc713` is the one place that records a house
change: byte +0x130 and the house, leader and family words. Its only
caller is `0x45a84b`, inside routine 17's `0x45a22d`. The main loop finds
the request between ticks and leaves the house. So a lot pick never
travels by itself. The script's request is the trip.

## Decision

1. **`Game` stops at a pending trip.**
   - `Game::trip_request` returns the request the lot holds, unless the
     host declined it.
   - While one is pending, no tick runs. `run_while` stops after the tick
     that asks and before the next one. `advance_while` runs nothing.
2. **The shell carries the trip out between ticks.**
   - `NeighborhoodShell::pump_trip_request` departs through
     `vm_travel::depart_from_request`. Routine 17 already wrote the tokens
     and the script paid the fare, so the fare is 0 and the party is the
     script's.
   - The window's loop calls it after each frame's ticks.
     `handle_lot_outcome` calls it after each input.
   - `depart_to_destination` and the trip path share one departure
     routine (`Departure::Request` and `Departure::Trip`).
3. **The lot-pick shortcut is gone.** `handle_special_panel_travel` is
   removed.
   - The pick is still recorded. The shell reads its picker only for how
     the trip goes, which picks the loading screen: the Magic Hole, the
     blimp, or the star limo. It does so only when the picked lot is the
     trip's house.
   - The record is dropped when the trip is handled, departed or refused,
     so the trip departs once and a stale pick never colours a later trip.
4. **Refusals.** In these cases the shell shows a notice and declines the
   trip (`Game::decline_trip`), and the lot goes on:
   - co-op;
   - already away on a trip;
   - editing a destination lot;
   - a house that is not a destination;
   - a failed departure.

   A house opened alone has no neighborhood, so the window declines its
   trips with a notice. A headless still (`--png`) reports the trip on
   stderr, declines it, and runs on to the still's tick.
5. **A declined trip stays declined.** `sims_vm` clears a request only on
   departure, and `Lot::world` is crate-private. `Game` therefore remembers
   the declined request and lets ticks run while the pending request is
   the same trip: the same house, leader, family and caller.
   - A later routine 17 does not replace a pending request (`generic.rs`,
     routine 17's merge). It only adds the neighbors it saved to
     `saved`, which the comparison leaves out.
   - So after a refusal no trip can be carried out until the lot is left
     or reloaded. A retry needs the VM to drop the request (below).

## The dialog release latch (`0x5e45e1`–`0x5e45ed`)

The latch is interface code and is already ported. A `cTSWinBtn` made by
`0x5e3844` starts with `+0xf4` = 0, so it acts on release. `+0x169`, which
the move handler at `0x5e4644` keeps, says whether the pointer is over it.
On release (`0x5e4563`), a button in state 0 or 2 that has the pointer
over it is published as the button released (`[0x6fe9a4]`, the
application's wait check), fires (`0x5e4c1e`), and plays its release
sound (`0x5e4855(2)`).

`Hud::release_dialog` answers only when the release is inside the pressed,
enabled button. The answer carries the button's bitmap
(`Answer::bitmap_id`), which the VM publishes as global 12. sims-vm needs
no change, as Track 4's ADR (`20261124-neighborhood-open-and-pools-parity`)
also concludes.

The phone test now answers the call's first VM dialog by mouse:
- a press dragged off the button and released does not answer;
- a press alone does not answer;
- a release inside answers, and the tick that asked runs again.

Making `release_dialog` ignore the pointer fails both phone tests.

## Evidence

`crates/sims-game/tests/phone_go_downtown.rs` (needs `SIMS_INSTALL`) has
two tests.

**From the phone to Downtown through the shell:**
- The household head calls; the test tries each phone until one routes.
- The two dialogs come next, the first answered by mouse.
- Lot 21 is clicked on the graphical Downtown map.
- Frames run as the window runs them.
- Once the trip is asked for, `run` and `advance` run nothing.
- The shell lands on lot 21 with:
  - the caller as leader and only traveler;
  - `fare_paid` 0 and the script's fare taken once;
  - the saved-state tokens (GUIDs and counts) routine 17 wrote on the
    home lot, unchanged;
  - the request carried out, and no lot pick left behind.

**A declined trip lets the lot play on.** On a house opened alone, the
pending trip stops the ticks, `decline_trip` lets 30 run, and the declined
request stays declined.

With the shortcut back, the first test fails with a fare of 50. Without the
stop between ticks, both fail ("a tick ran with a trip pending").

Measured on the Goth house: both the Go Downtown and Magic Hole scripts ask
for the trip one tick after the pick's replay tick. Both ask in mode 0,
for the caller alone.

Two tests changed, because both assumed the shortcut:
- `magic_town_arrival` expected the whole household to jump in. It now
  checks that the jumper goes alone and that the marker places them.
- `playtest_regressions` expected travel on the click itself. Both now run
  frames until the script asks for the trip.

## Not in this change

- **A trip home from a destination.** Routine 17 can name the home house.
  The shell refuses it ("Return home before…"), because
  `return_from_destination` does not take back routine 17's tokens. That
  belongs with the VM's return path.
- **Clearing a declined request in the lot.** `Lot` has no API to drop
  `World::trip_request`. A `Lot::cancel_trip_request` in sims-vm would let
  `Game` drop its declined copy, and let a Sim try again after a refusal.
  Whether native keeps or drops a request its main loop refuses is not
  read.
- **The transport in the saved trip.** `depart_from_request` records the
  default transport in `TravelSession` (the blimp or the studio shuttle).
  The loading screen uses the picker's, but a reload of a saved trip does
  not know it. Carrying it belongs in `sims_vm::travel`.
- **The needs bars' trend arrows.** The 800 tour shot shows red arrows on
  three bars. The draw is now read and documented in
  `sims_ui::needs_history`:
  - five 500 ms colour fades per bar;
  - steady colours `0xd10000` and `0x00ca39`;
  - a 3×5 triangle, 4 px apart, from the bar's two anchors.

  The sample's gate, the live update's flags & 9 (`0x51d05c`), has not been
  traced to where the flags are set. That gate sets how many arrows show.
- **Portraits.** The roster portraits score clean in the 800 and 1024 live
  shots. The only miss in them is the oracle's mouse cursor. Dialog,
  neighborhood-family and scrapbook portraits still have no scorable
  capture.
