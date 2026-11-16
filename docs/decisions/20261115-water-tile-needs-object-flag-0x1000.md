# A tile with water is a room's tile only under an object with flag 0x1000

*Decided 2026-11-15.*

The Roomies (House10) stopped at tick 2, where object 176's tree 0x0176
node 26 plans a route through a room with a tile of water on the ground.
The planner's tile test (0x4821f3), which says whether a person of a room
may stand on a tile, failed loudly there. It now does what the original
does:

- **Which tiles have water.** As before: a tile on the ground whose byte
  in lot array 10 is not 0xff (0x478f4a). Above the ground no tile has
  water.
- **The rest of the test (0x4822f8 to 0x482332).** A tile with water
  passes only if an object on it has flag 0x1000 in object data word 8
  (+0x5a, 0x48231d). The objects are walked as the lot links them
  (0x43f331 from the tile, then 0x43f379), the same walk the door test
  just before it uses (`placement::standing_on`). The first object with
  the flag passes the tile. A tile with water and no such object fails,
  so it becomes part of a hard rectangle of the room.
- **The lot.** The walk is given the lot through the global 0x6e48bc,
  where the door test uses the lot it was called with. The port has one
  lot, so the two are the same.

**Against the capture.** Track 2's capture `house10-roomies-ticks-20261115`
was run with answers 3, then 2, then four 2s, answered at ticks 14, 31
and 101 as the recording has them. The Roomies run 1000 updates:
- At every update, the random streams, the cursor, the action serial,
  the globals and the object IDs match.
- Every object's data, temps, attributes, frames and person data match,
  except person 176's person word 37 at updates 2 to 401 (3263 there, 0
  here). That word holds memory the original never set, as recorded for
  the Pleasants in
  `20261115-household-selection-works-out-again-who-visitor.md`.

**Strict runs.** The Goth and Newbie strict runs are unchanged. No
planner on those lots reads a room with a water tile, or it would have
failed loudly before.
