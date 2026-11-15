# A room upstairs, and K = 752

*Decided 2026-11-14.*

The planner's room rectangles (0x4818af) read the level from the room's
ID and take that level's room map and walls; nothing else in them
depends on the level but the ground's covering and water, which count
only on level 1. So routing upstairs is the ground's code with the
level's arrays, not a port of its own. A room ID naming level 3 or 4
fails as invalid state, as the lot holds two. K = 752: ticks 691 to 752
equal the fifth capture; tick 753 needs a pose with a carry layer.
