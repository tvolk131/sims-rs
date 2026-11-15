# The ambient scores play as the original's score player does

*Decided 2026-11-14.*

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
