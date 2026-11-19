# Grass darkens at night by the sun, not by room 0's light

*Decided 2026-11-18.*

At 12:24 AM the original's lawn was a brighter green than ours. Ours
dimmed the daytime grass by room 0's light, as every outdoor surface is
dimmed. The original's night grass pixels are undimmed palette colours,
shades 82 to 85 instead of about 13 by day: the sun has moved.

**The sun follows the time of day.** Signal 0xdd goes out at the end of
every load (0x416060) and on each change of the time of day. The panels'
handler passes global 4 to the grass (0x4c41bf). On an ordinary lot,
where neither of the grass's two preset flags is set, its override
(0x49e45e) hands global 4 to the sun setter (0x4a4551). The setter sets
the sun 1, 2, 3 or 2 tenths of the float π from the zenith for day,
dusk, night and dawn (doubles 0x668348, 0x667ac8, 0x668710), and 0 for
any other value. Then it shades the grass again. On flat ground the
shade is 13 by day, 44 at dusk and dawn, and 84 at night.

By day, ours now uses the setter's float, 0x3ea0d97c. The grass's
constructor sets 0x3ea0d973, 9 ulps less (0x49c187), and ours used to
keep that float. This holds only if the panels receive the load's 0xdd.
0x416060 shows that the load sends it, not that the panels have
subscribed by then. If they subscribe later, the grass keeps the
constructor's float until the next time-of-day change. The two floats
differ only on some slopes: flat ground is shade 13 with both, and on
the Goth house they draw the same, so no pinned render changes. A trace
of the first daytime draw on a slope where they differ would settle it.
At night the question doesn't arise: the time of day has changed since
the load, so 0xdd has reached the grass.

This corrects [20261114-sun-does-not-turn-camera](20261114-sun-does-not-turn-camera.md).
That entry read the setter's argument as the camera's rotation and
message 0xdd as a debug command. The argument is global 4, and 0xdd is
the time-of-day signal the sim already sends (`sims-vm` `tick.rs`). The
entry's finding still holds: turning the camera does not move the sun.

**Nothing else lights the grass.** The column drawer copies palette
blocks straight to the screen (0x49da21), and the blades are drawn the
same way (0x49d8fb). Ours had applied the light of the room at each
tile's middle to its grass (#68). By day room 0's light is full, so open
lawn changes only at dusk, night and dawn. A tile split by a diagonal
wall is the exception: its middle lies on the wall and can fall in the
indoor room, so the grass in its outdoor half was drawn in that room's
light, a dark dotted line along the foot of the Goth house's bay window.
That line is now the lawn's green.

**After dark, the drawing's light is capped at 0.8.** On the same
signal, the lot sets the brightest a lit tile may be drawn: 1 by day and
the float 0.8 otherwise (0x466ba3, at 0x6c741c). The drawing's light
setter holds the light it is given between 0.5 and that value (0x4b4525).
Ours held it between 0.5 and 1 at every hour. Now every light ours
passes through that setter takes the cap: floors, walls, objects, the
wall-top and corner trims, the strips that cap cut walls (patterns 252
and 253, which ours had drawn in full light; 0x53eba8 to 0x53ebb5),
shadows (whose strength reads the capped light, 0x4ae995), and a
person's level (`sims-people` `level`). The cap reaches ours from
global 4 directly; the original's follows the same 0xdd.

Some objects are lit at the cap whatever their room: those whose master
definition has word 69 (byte 0x8a) equal to 7, such as the pool's
diving board and ladder. After setting the tile's light, and the full
light of an object that glows, the original passes 1 through the setter
for them (0x437adb to 0x437aed). On an unlit patio at night that is 0.8,
where the room's light would give 0.5. A tile of a multi-tile object
reads its group's master (0x45728d).

A wall face and the cap at a wall's exposed end take their room's light
as a float (0x53dc78, 0x53ea46; 0x53c9d3, 0x53c9df), not the room's
tile byte. A light of 0.999 rounds to byte 255 but is not within a
millionth of 1, so the wall is shaded; ours had drawn it in full light.

The cap applies only to the light given to the setter. The original
also resets the light to full outright (0x4b455f), and that reset isn't
capped: the corner pass and the cap at a wall's exposed end use it when
their room is missing (0x53c3c6, 0x53c9e9). Ours does the same. The wall
pass, which draws the trim along a straight run between corner records,
does not: without room 0 it passes 1 through the setter (0x53dbb4,
0x53e2f5), so that trim takes the cap. Without a room on a wall's side,
the wall pass keeps the light of the last wall it read (first 1); ours
takes 1. Lamp pools add to the capped light and can pass 0.8, as
before. Person draw flags that force full light are not ported.

Full light is a light within a millionth of 1 (0x4c7c59, the float
0x358637bd), not exactly 1 as ours had it.

**Checked** against the original's 12:24 AM screenshot (near zoom,
rotation 0, walls down), pixels within 8 of the original:

| | Before | After |
|---|---|---|
| Whole overlap | 64.9% | 95.2% |
| Pixels green in the original | 12.1% | 98.9% |

Most of what is left is the original's interface overlays, the sleeping
Sim's head and a few cut-wall trims. No room in this shot is brighter
than 0.8, so the cap changes nothing in it. The pinned renders are all
by day. In the far and medium views, and the near view at rotation 3,
the bay window's dotted line changes (42 pixels at far zoom, rotation 0,
and 83 at medium). One flower pixel also changes by one step at medium
zoom, likely a blended edge over grass that changed.

**Not ported:** lots 28 to 29, 40 to 42, 46 to 48, 90 to 96 and 99
(global 10) take other terrain presets (0x4c4648). Those set the
grass's preset flags, and the override then uses other angles. The
preset for lots 90 to 94 uses 1, 1.5, 2 and 1.5 tenths of π; lot 99's
uses one tenth at every hour. Everything above is for ordinary lots.

**Unverified:**
- That the panels have subscribed by the time the load's 0xdd goes
  out, which decides the daytime sun angle (above).
- The cap has not been compared with a screenshot of a room brighter
  than 0.8 at night.
