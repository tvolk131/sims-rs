# A person upstairs stands on the terrain too

*Decided 2026-11-15.*

A person on a level above the ground now stands on the terrain's
altitude under their position, as a person on the ground does
(`sims_people::lot`). The original's model matrix (0x592132) looks up
the altitude (0x478dfe) whatever the person's level; it skips it only
for a point off the lot (0x45ee1d). It then adds the level's offset from
the view's table, which is still not ported: we keep projecting a level
as the lot renderer does, a floor height per level above the first. The
lot renderer lifts an upper level's floor by the terrain under it too,
so before this a person upstairs stood that much below their floor. On
the Goths' lot the house sits six terrain steps up, 48 pixels at medium
zoom, and the floor hid Bella completely while she played with the
dollhouse upstairs (ticks 798 to 1872, 6:56 to 7:32 AM).

Checked against a screenshot of the original at 7:29 AM (medium zoom,
rotation 0, level 2, walls down, camera origin (335, −496)): Bella
kneels at the dollhouse where we draw her at tick 1770, within a pixel
or two, with the same rows hidden by the front wall's strip. So the
view-table offset lifts a level by the floor height, at least at this
zoom and rotation. The original draws her brighter there; that is the
room's light (`20261115-person-lit-originals-standard-light-their-rooms.md`),
not her place.

This replaces what `20261115-people-join-lot-renderers-view-through-depth.md`
said of such a person: that they stand at the floor.
