# People play the simulation's clips; placement and facing

*Decided 2026-11-14.*

A person is drawn from the simulation's animator (`sims_sim::animator`),
read only:

- **Clips.** Every animation file (a `.bcf` that starts with eight zero
  bytes) is read once, and its clips are known by name, ignoring case. A
  clip's samples come from `<skill>.cfp`. A clip is posed at a phase from
  0 to 1: the frame is `phase × frames`, blended with the next frame,
  which wraps to the first only for a layer that repeats (end mode 1).
  This is the old prototype's sampling (`legacy/crates/sims-sim/src/pose/sampling.rs`)
  in ordinary `f32`. It is **unverified** against the original's frames.
- **Layers.** The rest pose, then the base layer's clip, then the
  foreground layer at its phase and weight. The simulation keeps only
  whether a base layer exists, not its clip or phase. Until it exposes
  them, the clip comes from the saved person record (the base descriptor,
  for the Goths `a2o-standing-loop` and `c2o-standing-loop`) and the phase
  from the animator's clock. Carry and blend layers are not drawn yet.
- **Root travel.** The walking clip's root sways forward and back by
  about 0.27 units, and returns; it does not travel. The simulation moves
  the person. So the clip's root is kept as it is, unlike the old
  prototype, which removed it.
- **Place.** Sim x and y (sixteenths of a tile) become model X and Z, at
  3 model units per tile. That figure comes from the walk step, which
  moves a person by a clip's distance times 16/3 sixteenths. Floors above
  the ground are not handled yet.
- **Facing.** Checked against the simulation, not the original: person
  259 walking for 82 ticks of `sims boot` on House05 moves along
  (sin θ, −cos θ) in sim x and y, where θ is its direction (in eighths of
  a turn) times 45 degrees. The mean difference is 0.2 degrees. Every
  clip leaves the toes pointing at model +Z, so a person is turned by
  π − θ about the vertical, as the old prototype did. The turn is computed
  with additions, multiplications and square roots only, so pinned renders
  hold on every machine.

Whether the drawing is the original's image or its mirror image is still
**unverified**, and so is which screen direction sim +x takes. Both wait
on a screenshot of the original.
