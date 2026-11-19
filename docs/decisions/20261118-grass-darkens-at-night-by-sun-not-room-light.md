# Grass darkens at night by the sun, not by room 0's light

*Decided 2026-11-18.*

At 12:24 AM the original's lawn was a brighter green than ours. Ours
dimmed the daytime grass by room 0's light, as every outdoor surface is
dimmed. The original's night grass pixels are undimmed palette colours,
shades 82 to 85 instead of about 13 by day: the sun has moved.

**The sun follows the time of day.** Each change of the time of day
sends signal 0xdd. The panels' handler passes global 4 to the grass's
sun setter (0x4c41bf calling 0x4a4551). The setter sets the sun 1, 2, 3
or 2 tenths of the float π from the zenith for day, dusk, night and dawn
(doubles 0x668348, 0x667ac8, 0x668710), and 0 for any other value. Then
it shades the grass again. On flat ground the shade is 13 by day, 44 at
dusk and dawn, and 84 at night.

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

**After dark, nothing is drawn brighter than 0.8.** On the same signal,
the lot sets the brightest a lit tile may be drawn: 1 by day and the
float 0.8 otherwise (0x466ba3, at 0x6c741c). The drawing's light setter
holds every light between 0.5 and that value (0x4b4525). Ours held it
between 0.5 and 1 at every hour. Now floors, walls, objects, trims,
shadows and people (`sims-people` `level`) all use the cap. After dark,
then, nothing is drawn in full light, since full light needs exactly 1.
The load sends its own 0xdd once the house is read, so the cap always
matches global 4.

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

**Unverified:**
- By day ours keeps the constructor's float, 0x3ea0d973 (0x49c187). The
  setter's tenth of π is 0x3ea0d97c, 9 ulps more. The original holds the
  setter's float once the time of day has changed since the lot was
  loaded, or from the load on if the load's own 0xdd reaches the panels.
  The Goth house's grass at far zoom, rotation 0, draws the same either
  way.
- The cap has not been compared with a screenshot of a room brighter
  than 0.8 at night.
