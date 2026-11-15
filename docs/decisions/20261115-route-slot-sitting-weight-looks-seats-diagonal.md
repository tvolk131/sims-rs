# A route to a slot with a sitting weight looks for seats, and a diagonal slot turns through a matrix

*Decided 2026-11-15.*

**Unverified.** The search by distance (0x483598) now handles a slot with
a sitting weight. It tries each place as indices -1 to 8: first the place
itself, when standing weighs at least as much as sitting; then a seat one
tile away in each of the eight directions (0x4838e7, 0x422632, 0x43244d);
and last the place itself again, when standing weighs less. A seat must
offer Sit (function 26), face back toward the place, and have that
direction's bit in definition word 66. If someone else holds the seat,
result 9 is noted and the holder goes in context word +0x8c. A candidate
beside a seat ends the route with the seat's Sit tree, not a walk
(0x45c430, 0x45b96b), and the route then waits for that tree's return
(step 4, 0x45b3fa).

A slot place turned by a direction other than 0, 2, 4 or 6 goes through
the original's rotation matrix in world units (0x483d7c, 0x455070,
0x5c4510, 0x454fb9). The cosine and sine come from libm, so they may
differ from the x87 `fsin`/`fcos` in the last bit of a float. Seats for a
person of kind above 1 still fail loudly (0x46de84), as does a slot with
flag 0x10000.

This moves a player's snack order on the Goth house from tick 154 to
tick 615. There it stops at Create New Object Instance mode 4. Its seat
search picks a place to stand, so the Sit route is covered only by
synthetic tests. Reopen when a capture of a route that sits disagrees,
or when a rounding at x.5 after the matrix lands on the other side.
