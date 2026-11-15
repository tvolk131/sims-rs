# The lot notes its sound requests; sound changes nothing it keeps

*Decided 2026-11-15.*

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
  object's reset (0x4320b4) and the destructor (0x4323c9). (This first
  said a reset for a deletion stops nothing; it does, corrected in the
  entry for tick 3602.) A person's reset makes no direct call.

In 870 play ticks the Goth house plays 62 sounds and stops 50:
begin_route 21, end_route 18, door_open 4, and so on.

The sounds a clip's events play (footsteps, "sound" keys, 0x59175e) are
not noted yet. They pick their sound from the person's state and the
floor in code that is not ported; that is the next sound work. The
hour for the ambience is global 0. The camera view is the interface's.
