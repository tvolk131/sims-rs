# The game draws people over the lot with a matching 3D camera

*Decided 2026-11-15.*

Superseded by "The game draws people into the lot with sims-people's
depth test" above.

`sims-game` draws the people with `sims-people`'s orthographic camera,
built from the lot camera so that a ground point lands on the pixel the
lot's 2:1 projection gives it (within the lot camera's rounding, at every
zoom and rotation; pinned by a test). Heights take the scale a
horizontal length gets; a person upstairs is raised one storey's
screen height, and every person by the terrain under them (the tile's
corners blended by where in the tile they stand). The people are drawn into their own image and laid over
the lot whole, so walls and furniture in front of a person do not hide
it yet; occlusion belongs to the render and people tracks. That the
original draws its people at this scale is **unverified**.
