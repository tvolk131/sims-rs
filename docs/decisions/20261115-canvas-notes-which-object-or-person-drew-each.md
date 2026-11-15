# The canvas notes which object or person drew each pixel

*Decided 2026-11-15.*

For the game window's clicks, `sims_render::Canvas` keeps an ID per
pixel beside its colour and depth. Whatever draws sets a pen
(`Canvas::set_id`); every pixel the canvas takes after that, by its
usual rule (passing the depth test, not fainter than `FAINT`, blended
or not), takes the pen's ID. `Canvas::id_at(x, y)` gives it back:
`Option<u16>`, the key in `World::objects`. `sims_render::draw` clears
every ID and gives object sprites their object's ID and terrain,
floors, walls, trims and caps none. `sims_people::People::draw_on_lot`
(and `People::draw` into any `Target` that keeps IDs, through the new
`Target::set_id`, a no-op by default) gives each person's pixels the
person's object ID. So after the lot and its people are drawn, `id_at`
names what is shown at a pixel; an object or person hidden behind a
wall is named nowhere, and a pixel a soft edge blended over names the
edge's owner. Each tile of a multi-tile object is its own object and
names itself; finding its lead object is the caller's. Nothing changes
what is drawn: the Goth house digests are unchanged.

For `sims-game` (track 1): its people loop (`Marked`, `shown` in
`src/people.rs`) can set the canvas's pen per person (or call
`draw_on_lot`) and read `Canvas::id_at`, and `src/pick.rs` can pick
objects by it instead of by tile. Whether to search nearby pixels when
the pixel itself names nobody (`REACH` today) is the game's choice.

How the original resolves a click to an object or a Sim (by an ID
buffer, as here, or otherwise) is not read; this is ours, and what it
names should be compared with the original's choice of object at the
same pixel once the harness can click.
