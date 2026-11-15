# Job data from the CARR chunks, a frame after tick 901, and K = 991

*Decided 2026-11-14.*

Scope 33 reads a field of a career level from the careers `work.iff`
holds, as the original's list is built from its CARR chunks (0x403e3f)
and read by 0x40362f; the field order is ported from that code, and
field 22 (whether the level's second string is not empty) takes the
string's first virtual function as its length, as the old prototype did:
**unverified**, reopen if a script reads field 22. An install with a
`Careers.txt` still fails loudly. The fifth capture's room motives show
that it drew a frame after tick 901, two ticks off the schedule the
earlier captures share, so the comparison draws one there. The schedule
is the original's main loop, not the simulation, so the default
(`0,1,2,3,33+30`) is unchanged; reopen if a capture can record its draws,
which would replace the inferred schedule. K = 991; tick 992 snaps to a
slot whose placement checks the border, the room and the walls.
