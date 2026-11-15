# The game draws people into the lot with sims-people's depth test

*Decided 2026-11-15.*

`sims-game` no longer draws the people into an image of its own laid
over the lot. Each frame it draws the lot with `sims_render::draw`, then
the people with `sims_people::People::draw_on_lot` into the same canvas,
which tests every pixel against the lot's depth, then the panel. Walls,
furniture and the floor above now hide the people behind them, at every
zoom and rotation; placement, terrain height and the light are
`sims-people`'s. People on a level above the camera's are not drawn,
and the camera's level stays the player's choice (Page Up and Page
Down), so on the second floor the people downstairs are hidden with
their rooms. How the original gives a person's mesh its depth is
**unverified** (see "People's meshes go into the lot renderer's canvas").

A click finds the person shown at that pixel in the frame on screen (or
the one shown nearest within 2 pixels): while it draws the people, the
game notes whose pixel the canvas took last at each point. A person
hidden behind a bookcase is shown nowhere, so it no longer takes a click
meant for the bookcase, and one off the lot or upstairs of the view is
not found. `sims-people` does not say which person drew a pixel, so the
game repeats `draw_on_lot`'s loop from its public parts (`posed`,
`skin`, `draw_part`, `View`); a test checks that the frames are the
same, colour and depth. Reopen when `sims-people` reports the person
per pixel.
