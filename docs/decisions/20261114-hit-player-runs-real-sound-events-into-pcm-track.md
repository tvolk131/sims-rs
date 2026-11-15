# The HIT player runs real sound events into PCM (track 3, sound)

*Decided 2026-11-14.*

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
