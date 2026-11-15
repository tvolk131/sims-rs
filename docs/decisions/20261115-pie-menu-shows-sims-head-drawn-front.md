# The pie menu shows the Sim's head, drawn from in front

*Decided 2026-11-15.*

The original renders the selected Sim's head live into the middle of the
pie, in the blank face bitmap (`sims_ui::pie`, kPieFaceBkg, 201 by 201).
`sims-game` now does the same with `sims-people`'s public parts
(`People::head`). The Sim is posed as for the lot (`People::posed`). The
parts bound to the `HEAD` bone are skinned, which gives the head with its
hair, and glasses where a Sim wears them. The software rasterizer draws
them into a clear image the size of the face bitmap.

- The camera looks the face in the eye, 10 degrees from above. It is
  centred on the head and scaled so the head fills 80% of the face
  across or down. The light comes from the front and above. All of this
  is our choice. The original's camera, light, size and any animation
  of the head there (does it follow the pointer, or talk?) were not
  read, so the head is **not** compared with the original's.
- The face looks along the root's local +Z. This was found by drawing
  Mortimer: with +X his right side showed.
- The head is drawn again with every frame while the menu is open, from
  the pose of the moment. That poses everyone on the lot a second time
  for the frame. It only happens while a menu is open.
- A Sim who cannot be posed (not placed, no `HEAD` bone) gets no head.
  The face bitmap shows alone, and the problem is listed once among the
  people's problems.
