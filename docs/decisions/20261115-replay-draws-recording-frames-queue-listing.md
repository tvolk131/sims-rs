# The snack order's replay draws the recording's frames; a frame lists the selected Sim's queue

*Decided 2026-11-15.*

The replayed snack order (`goth-snack-order-20261115`) differed from update
454 in the people's Room motive, in spells of 18 updates, then in
Mortimer's data word 50 (from 622) and his running action (from 892).
The Room motive and the running action came from the frames the original
drew between updates, not from the simulation.

- **The frames.** A person's Room motive takes the room's score each tick
  (0x45cef4), and a room is counted again only when a frame is drawn
  (0x467094). The daylight changed in update 453 on both sides (object
  252's local goes 2 to 1). Our window drew its frames on the schedule of
  the captures without input (after 33 and every 30th update from there,
  `Frames::default`), so room 4 was counted after 453 and Bella's motive
  took the new score (61.757) in 454. The recording has it in 472, and the
  other people's in 532. That fits a frame after every 30th update from
  171: the same period, 18 updates later. With that schedule
  (`0,1,2,3,33,63,93,123,153,171+30`) every person's motives match the
  recording to update 1000. When the original draws a frame is up to its
  main loop, so the schedule is an input of the recording, like the
  pointer. `sims_game::Options` now takes it (`frames`; the window keeps
  the default).
- **The queue** (read from the code). `DDDSimsView::Draw()` (0x4d1b37)
  brings the control panel's action queue up to date (0x4d28df) before it
  draws the house view. In live mode (the panel's mode word is 2: the
  mode switch, 0x4cea5d, logs "Entering Live Mode" for 2, and Build 0,
  Buy 1, Camera 4, Options the rest), it lists the
  selected Sim's running action and then the queued ones, each whose icon
  object is set (0x4ea31c, 0x4ea203). Copying each into the list works out
  its name (0x4e87b7 → 0x41f167), which clears the action's flag 0x80 (the
  flag set when an action is named). `Interpreter::draw_frame_in` now
  does this first in live mode, and in no other (`PanelMode`);
  `draw_frame`, which the window uses, is a live-mode frame. Mortimer's action chosen in update 872 (routine 4170 on
  object 263) has flags 0x9e; the recording has 0x1e from update 892,
  after the frame that follows 891.
- **Verified**: with the schedule and the queue, the window's lot equals
  the recording's after every update from 1 to 1000, except:
  - update 33 (person 259's animator) and update 891 (the running
    action's flag 0x80). The capture saves an update's state before the
    frame after it; `Lot::step` gives the state after that frame.
  - Bella's local in tree 8292 from 203 to 227, as before.
  - Mortimer's data word 50, 1 in the recording from update 622 to 871 and
    0 in ours.

  The free run without input matches the long captures as before, with
  and without the queue step.
  The strict boot's hashes at ticks 3432, 3602, 3603, 3912, 9906 and
  31478 changed (`tests/boot.rs`); those at 2881, 20186 and 43200 did
  not, nor did its text outputs at 1102, 3602 and 9906. The frame after
  2913 clears flag 0x80 on Mortimer's action (routine 4102 on object 8,
  icon 334): 220 becomes 92. The goth day capture has 92 for that action
  from 2930 to 3350, where main had 220.
  `the_recorded_snack_order_replayed_in_the_window_matches_the_recording`
  now runs to update 1000 and pins Bella's Room motive at 471 and 472 and
  the flag at 890 and 891.
- **The pie in the recording is not off centre.** The earlier entry noted
  that the original's pie sits 24 pixels right of the press, where ours is
  centred. The press was at (176, 198), and the menu reaches 200 pixels
  left of its middle ("Serve Breakfast", 136 wide, plus the radius and the
  margin). So the window is moved onto the screen (0x5ebb10, already
  ported) and its middle lands at x 200. Our layout gives the recording's
  window origin (0, 86) and all four label boxes to the pixel
  (`a_menu_near_the_edge_moves_onto_the_screen_as_the_original_moved_it`).
  Our pie looked centred because our window shows the fridge at (270, 206),
  where it needs no moving. The real difference is the view: in this
  recording the original's camera is 94 pixels left and 8 up of the
  window's at 800 by 600. Which camera the original opens the Goths' house
  with when the scenario runs is **unverified**; the render track's
  screenshots matched origin [367, -130].
- **Unverified**:
  - The frames before update 171. The recording shows nothing that tells
    them apart; the test keeps the default's.
  - The writer of Mortimer's data word 50 in update 622 (cleared before
    each check and set by check trees, `autonomy::data::CHECKED`). His own
    frames do not move then, and the running action's flags keep no 0x10,
    so neither his thread nor a check of that action wrote it. A pie
    listing on whatever came under the still pointer at (200,123) is one
    candidate. A recording with a write watch on it
    (`--watch d50=object:21+0xae:2`) will tell.
