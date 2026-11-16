# A press lists the pie menu again, and reaching an entry checks it again

*Decided 2026-11-15.*

Track 2's recording of the original ordering Mortimer's snack with the
pointer (the goth-snack scenario: the pointer reached the fridge before
update 124, pressed before 154, reached "Have A Snack" on the pie before
176 and chose it before 202) was replayed through the window's input
layer, in strict mode, with the same input between the same ticks, and
our full state after every update compared with the recording's (the
tick verifier, from a throwaway dump of the window's lot). Two input
steps did not do what the original does; both are fixed:

- **A press lists the menu again.** The live mode's press handler
  (0x48cbec, vtable 0x667f94) first sets the object under the pointer
  to none (0x48c834 with 0), so its own pick lists the menu anew
  (0x48c834 skips only the object it has). The window used to open the
  listing made when the pointer reached the object. In the recording
  Mortimer's last result goes false again in update 154, as a listing
  leaves it. `Game::press` now lists at every press on the lot.
- **Reaching an entry checks its action again.** The menu window's
  handler (0x51bb9e) calls 0x48df28 for the entry the pointer is on: a
  context made by 0x458293 with the byte 1 (put back by 0x4582dd) and
  0x458319 on the entry's action with the advertisement record as its
  output. That is autonomy's form of the check: the arguments 1, 0, 0,
  0, the target's data word 25, the entry's advertisements copied over
  the record, and the room rule (0x45853f, only with an output).
  0x48df28 then scores the record for the interface (0x4214a1).
  `Lot::highlight` (`Interpreter::highlight`, `Machine::highlight`)
  runs it, and the window calls it each time the pointer reaches a
  choice's entry of an open menu (`Game::highlight`). In the recording
  Mortimer's last result goes true in update 176 (the snack's check
  passes), and the locals the snack's frames start with are what Have A
  Snack's check tree (0x1008) left in his stack, not the listing's last
  entry's (0x1030).

**Verified** (the recording, to update 941): with both fixes the
window's lot equals the recording's after every update from 1 to 453
in everything the verifier compares, but for two things the input does
not change. Person 259's animator is marked drawn at update 33 where
the original's is not, and in a free run of the window against the long
capture too: the window draws every tick. Bella's (object 18) frame of
tree 8292 starts with a local of 16104 at update 203 where ours is 0,
until update 228; the free long capture has 0 there. From update 454
the people's Room motive differs in spells of 18 updates (454, 514,
604), then Mortimer's data word 50 (from 622) and his running action
(from 892). The input steps are done by then, and the same happens
without the entry check, so that is the simulation after the snack,
not the input. `the_recorded_snack_order_replayed_in_the_window_matches_the_recording`
pins what the recording showed the input doing.

**Unverified**: the message that makes the menu window call 0x48df28
(the recording shows only that it ran when the pointer reached the
entry), whether a pointer that moves within one entry checks it again
(the window checks once per entry reached; leaving the window forgets
the entry, so coming back onto it checks it again), and whether a check
tree's Add/Change Action String adds anything then (taken as in
autonomy's check: no menu is being built). The original's pie in the recording
sits 24 pixels to the right of the press, where ours is centred on it.
This supersedes "A click opens that listing" in the entry "The game
lists the pie menu on the lot when the pointer reaches an object".
