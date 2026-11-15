# Bella Goth compared with the original; the mirror is still open

*Decided 2026-11-14.*

One pose was compared with the original game. The game ran in the
emulator harness (`tools/harness/`, its own container and guest copy),
with House05 loaded from the install and the house view at near zoom.
Bella (person 18) was walking away from the camera, and a screenshot was
taken from behind at about 27 pixels per model unit. `sims-people` then
drew person 18 from the same saved outfit at the same scale, playing
`a2o-walking-loop` at directions 3.5 to 4.5 and phases 0.05 to 0.8. The
screenshots and renders stay private, outside the repository.

What matches: the meshes (body, head, long hair, hands, shoes), the
textures (the red dress with its open back, the skin tone, the hair), the
proportions, and the hem's length. The pose falls inside the walk cycle:
it looks like phases 0.05 to 0.3.

What the comparison does not settle:

- **Mirror image: unverified.** The drawing negates file X (see
  `crate::rig`). A capture from behind has no fixed feature that is
  different on the left and right. The hem looks like one, but drawn at
  several phases its lower side flips with the stride, and the
  screenshot's phase is not known. Settling it needs a fixed asymmetric
  feature seen in the original: for example, a printed shirt or a hair
  parting, seen from the front.
- Light and shading: the renderer uses its own light, not the
  original's.
- Pixel positions: the camera was not matched to the original's pixel
  for pixel.
- The standing loop, Mortimer (person 21) and Cassandra (person 16, a
  child): the capture showed neither of them in their saved outfits.

Mortimer was also seen walking, in a work uniform (a lab coat). That
outfit is not among his saved outfits and was not found in the install,
so it was not compared. Looking for it showed that an appearance's own
name does not always match its file's name (`B629MAFitlgt_labtech` is in
`b629ma_labtech.cmx`). So appearances are now also found by their own
names across every catalog, as the old prototype did
(legacy/crates/sims-content/src/avatar.rs).
