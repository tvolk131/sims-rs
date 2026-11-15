# Two more cameras compared with the original

*Decided 2026-11-14.*

The original was shot at two more cameras, walls up, after the load's
notices were answered and the game was paused. Each was compared with
our drawing of the world after one tick of `sims boot`, with the
camera's origin fitted by search. The screenshots stay private.

**Medium zoom, rotation 0** (one zoom step in, then panned; origin
[463, -496]). Exact matches:

- Whole screen: 76.4% (72.2% of the pixels that are not grass).
- Open grass (x 520 to 780, y 380 to 480): 96.0%. The grass rule read
  at far zoom holds at medium zoom.
- The house (x 0 to 720, y 0 to 390): 74.1%. Walls, wallpaper and the
  wood trim match pixel for pixel where they were checked.

What differs is the objects. Doors, windows, shrubs, the bench and the
stairs match only about half their pixels. A door shows both kinds of
difference: its frame's left edge is two pixels wider in the original
(ours is hidden behind the wall, which is in front of it by depth), and
its glass sits one pixel further right. So at medium zoom both the
depth of objects set into walls and the placement of some draw-group
parts are off by a pixel or two. Neither is explained yet. The fence,
drawn as an object too, matches 97.6%, so this is not a shift of
every object.

**Far zoom, rotation 3** (one turn of the panel's right-hand rotate
button from rotation 0; origin [575, -203]). Exact matches:

- Non-grass pixels: 55.2%. The house (x 230 to 480, y 200 to 400):
  55.2%, most of the loss being grass inside the box. Walls, trim,
  floors and furniture line up.
- Open grass (x 560 to 760, y 440 to 540): 12.7%. **The grass does not
  match at this rotation.** The shading has the same shape, but its
  dark streaks sit in other places, and the blades differ. Counting the
  vertex jitter and the blade map from view tile 0, or from either
  rotated lot corner, instead of from the lot's least view corner,
  reaches only 16%. The cause is **unverified**: the shading, the
  jitter's coordinates or the blade map may each turn differently in
  the original. Reading how it turns the heights (0x479567) and the sun
  (0x4a4551) is the next step.

**Seen at both cameras:** lamps are missing. The floor lamp in the
upstairs-left room and the kitchen counter's lamp (also missing at the
first camera) are drawn by the original and not by us. Lamps may be
drawn through a sprite their scripts switch on, as other dynamic
sprites are; that is unverified.

**Unverified:** near zoom, rotations 1 and 2, and the cutaway at these
cameras (the screenshot at medium zoom had no wall cut, so it adds
nothing to the cutaway check).
