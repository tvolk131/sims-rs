# The hour turns, a new time of day grows the lawn, and K = 900

*Decided 2026-11-14.*

The hour's turn is ported from the lot update (0x46d3cc to 0x46d51d).
When the time of day changes, the game tells the interface (0x4c4648)
and sends signals 0xf8 and 0xdd. The interface call picks a panel's look.
On 0xdd the lot's subscriber marks every room and sets a light ceiling
only the drawing reads.

Signal 0xf8 is not presentation, as a first reading had it (the review
caught this). The architecture's subscriber (0x47b22a) takes it and,
when global 36 is not 0, grows the lawn (0x47b3c8): each tile inside the
lot's border whose grass (Arry 6) is below its target (Arry 7), and that
has a diagonal neighbour already at its target, grows by 20, up to the
target. The sweep changes the array as it goes, row by row, and adds in
32 bits as the original does. It then sends 0x86 and 0x83: the
interface's panels (0x4c3ea5) set a refresh flag on 0x86, and the house
viewer redraws on 0x83. This is ported (`World::grow_grass`) from the
code; the original's direction table was read from a memory image. If
the lot lacks the two arrays the update fails before the time of day or
the rooms change.

Only the architecture takes 0xf8. Besides the lot, the interface's
panels take 0xdd (0x4c4189: the current mode's time-of-day look, then
0x83). That the panels' and the house viewer's handling of these signals
changes nothing the simulation keeps is **unverified** (the review
corrected an earlier, wrong census).

Ticks 878 to 900 cross 7:00 but do not change the time of day: the save
already holds day (global 4 is 0 at 6:30, in the capture and here), so
neither the lawn nor the rooms change, and they equal the fifth capture.
The first change comes at 18:00. The tick captures do not hold the lot's
arrays, so the lawn's growth is **unverified** against the original;
the full day's recording must capture the grass array at 18:00. The
day's turn still fails loudly. K = 900; tick 901 reads job data
(scope 33).
