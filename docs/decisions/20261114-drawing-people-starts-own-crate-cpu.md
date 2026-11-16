# Drawing people starts in its own crate, on the CPU

*Decided 2026-11-14.*

The Sims are drawn by a new crate, `sims-people`. It reads simulation
state and never changes it, and nothing in the simulation depends on it.
Its math is ordinary `f32`, and it may use the platform's `sin` and `cos`
(it allows the workspace's float lint at its crate root).

Terms used here. An *appearance* is a named set of meshes bound to bones
(`b004fafit_01`, Bella Goth's body). A *texture string* is what a saved
person keeps next to each appearance, such as `BODY=b004fafitmed_fatale01`.

What was decided:

- **Files are found by their own name.** Every `.bcf`, `.cmx`, `.bmf`,
  `.skn`, `.bmp` and `.cfp` file, loose or in a FAR archive, is known by
  its name without directory, ignoring case. The base game is searched
  first, then the expansions in the order the original mounts their
  object folders, then `Downloads`. The original's own avatar registry
  is not ported, so this order is **unverified**. Every file the Goth
  family is drawn from has one source or identical copies, so the order
  does not change what they look like.
- **What a person wears comes from its saved record.** The body, the
  two hands, the head and the accessories each name an appearance and a
  texture string. An appearance is read from `<name>.cmx.bcf`, or from
  the text form `<name>.cmx`. A mesh is read from `<name>.bmf`, or from
  `<name>.skn`.
- **A texture string `KEY=texture` gives its texture to every mesh whose
  name contains `KEY`.** Other meshes keep the texture their own file
  names (Cassandra's glasses, `nerdspecs`). This is the old prototype's
  reading of the original's code (0x599e87). It explains every part of
  the three Goths.
- **The pose is computed in a mirrored frame.** Positions and rotations
  from the files have their first coordinate negated, as the old
  prototype's renderer did. Whether this gives the original's image or
  its mirror image is **unverified** until it is compared with a
  screenshot of the original.
- **Skinning follows the old prototype:** each vertex moves with its
  bone, and a blended vertex moves part of the way towards its position
  on a second bone. How close this is to the original's blend is
  **unverified**.
- **Rendering is a small software rasterizer for now.** Tests draw
  offscreen with no window and no GPU. The camera keeps sines and
  cosines, and the house view is built from square roots, so a render
  gives the same bits on every machine and can be pinned by digest.
- **The texture decoder (`.bmp`) lives in `sims-people`**, not in
  `sims-formats`, to avoid conflicts with the rendering track. It can
  move later.

How it was checked:

- The three Goths (Mortimer, Bella and Cassandra) load with the meshes
  and textures listed in `tests/goth_family.rs`, and draw in their rest
  pose. The renders were looked at: clothes, skin, hands and heads sit
  where they should, and Cassandra wears her glasses. They are pinned by
  digest, gated on `SIMS_INSTALL`.
- Nothing is compared with the original yet.
