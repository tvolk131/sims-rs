# People join the lot renderer's view through a depth hook

*Decided 2026-11-15.*

sims-people draws a live lot's people in sims-render's camera rather
than its own (`sims_people::People`, `View`). It depends on sims-render,
and not the other way round, so the render track can take people later
through a trait of its own. Placement: a person's fine position (x, y)
sits at world (−3y/16, 0, −3x/16), and a unit of height is 32√6/3 ≈ 26.13
near-zoom pixels, from the original's house camera (0x49b0de: tilted
30°, 3√2/(16 << zoom) units to a pixel). Checked: a person's root
projects to the pixel the renderer's `project_fine` gives, at every zoom,
rotation and level, for elevations that are whole pixels of the zoom;
one step of altitude lifts a person exactly as it lifts the renderer's
ground (16 near-zoom pixels). The mesh is projected without rounding, so
at other elevations a point can fall a pixel above where `project_fine`,
which rounds the elevation on its own, would put it. In the booted Goths'
lot each person, drawn alone, is drawn where the renderer projects their
position, at all four rotations. People are drawn one
column right, as the renderer draws objects; **unverified** for people.

A person stands on the terrain under their position, found as the
original finds it (0x478dfe, the same lookup as `sims_vm`'s head model,
which is private, so it is repeated here). On an upper level the original
adds an offset from the view's table, which is not ported: such a person
stands at the floor. A person whose object is off the lot (taken off, at
tile −1, −1) is not drawn, wherever their animator was left;
**unverified** against the original.

Depth is the caller's. `People::draw` takes a function from a point's
fine view position, elevation and level to a depth, the signature agreed
with the render track for its coming `Depth::point`, and interpolates it
across each triangle; people are drawn far to near by their root's depth.
How the original gives a person's mesh its depth is **unverified**; reopen
when it is read. Until the renderer takes people, an app draws them over
the lot, and they overlap walls and furniture wrongly.
