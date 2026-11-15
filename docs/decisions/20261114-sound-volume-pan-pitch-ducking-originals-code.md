# Sound volume, pan, pitch and ducking, from the original's code

*Decided 2026-11-14.*

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
