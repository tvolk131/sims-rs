# People in the way in the person vector's order, and K = 436

*Decided 2026-11-14.*

Tick 418 of the Goth house stopped where a route asked who is in its
way with two candidates (`0x45bb04`). We failed loudly there because
the lot's list of people was thought not to be modeled.

- **The list is the person vector.** `0x45ba28` walks
  `[person +0xdc] +0x84`, the same vector Set To Next's people modes
  walk (`0x44efb4`, `0x44f0d4`). The world keeps it as `World::people`:
  the loaded people in registration order, then each person made since.
  So the candidates are taken in that order, and any number of them is
  allowed. Steps 2 and 3 of the search find the first match in that
  order, as the original's static candidate vector does.
- **The branch into the route is the original's.** Our run takes door
  52's tree 4105 to Go To Routing Slot because the door's graphic (data
  word 0) is 4. The capture's door holds 4 as well, so the original's
  run took the same branch.
- **Tick 418 itself.** The candidates are persons 18 and 16. Person 18
  has a way but is not asked, because 18's last route context waits on
  person 21. Person 16 has no way. Nobody is in the way, so here the
  order does not change the outcome.
- **K = 436.** The state after ticks 418 to 436 equals the capture, but
  for the known last-place head fields. Tick 437 stops where person 16's
  animator finds its next step blocked (`0x58f682`).

Reopen if a capture shows people in the way taken in another order than
the person vector's.
