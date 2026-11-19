# Burn places fire objects on a tile and spreads them to what catches

*Decided 2026-11-15.*

Burn (primitive 9) was the most used primitive the port lacked after
Reach. The base game's trees use it 11 times: the fire's own spread
("spread", "spread to object"), the stoves' cooking, the fireplaces, the
grill and the dragon. A kitchen fire reaches it. It is now ported
(`primitives/burn.rs`) from the general table (0x45246a), which sends
opcode 9 to 0x44d96f.

- **Operands.** Byte 0 is 1 for the tile in front of the stack object
  (the fire's spread uses this) and anything else for where it stands.
  Bit 0 of byte 1 (the stoves set it) lets fire take objects that
  data word 8 flag 0x20 or data word 62 would protect.
- **Where.** The stack object (missing: native error 0x15), standing on
  the lot (0x432595) or the false branch. In front adds one tile's step
  in its direction (0x42228a). A tile below 1, or at least the lot's size
  less 1, takes the false branch.
- **In front, the walls.** The wall on the side facing back at the stack
  object is bit 3 turned a quarter for each step of its direction halved:
  the table at 0x6f4670, which the image holds as zeros and 0x53ba5f
  fills. That wall, if the tile has it, must have a style a person sees
  through (3, 5, 6, 0xf, 0x17). A diagonal wall takes the false branch.
- **The tile's objects** (0x43f331, 0x43f379). One with data word 8 flag
  0x800, or a fire (type 0x24C95F99, the "Fire" object), takes the false
  branch. The last with data word 40 flag 0x20 catches. If none does, one
  fire is made and placed at the exact place.
- **The spread.** From what catches, the objects are gathered in the
  original's order, the gathering starting over after each addition:
  - the tiles of its group, unless its type has a tree for function 8;
  - what catches on its tile;
  - on each of the eight tiles around, the first object whose footprint
    overlaps its tile, a person's widened by 2.
  Any gathered object with data word 8 flag 0x800 takes the false branch,
  and so, without byte 1 bit 0, does one with flag 0x20 or data word 62
  above 0. Then each one whose tile has no fire gets one at its tile's
  centre. The place test takes the object's level as it was, and the put
  reads it again (0x44e065, 0x44e06f): a callback that moved the object
  during the test takes the fire with it. The branch is true when any
  fire was placed.
- **The footprint's order.** The original keeps a footprint at +0x100 as
  the greatest coordinates, then the least (0x433d16); the port keeps
  them the other way round. Burn's neighbour test (0x44de31) is an
  overlap test once that is accounted for.

**Fails loudly:**
- a gathered person, whom the original moves to its tile's centre
  (virtual functions 0x38, 0x58 and 0x74). Ported since; see
  `20261118-fire-burns-person-resets-whoever-uses-what-burns.md`;
- a fire the factory cannot make. The original takes the false branch
  when the factory gives 0, but the port's factory faults instead;
- a wall style the original reads from memory it never set (0x4795c5);
- a content without the fire type;
- a direction past the wall table.

**Unverified.** No capture covers a Burn yet. The tests are synthetic.
A recording with a kitchen fire should check where the fires go and in
what order they are made.

**Strict runs.** The Goth and Newbie strict runs are unchanged. Neither
reaches a Burn, or it would have stopped on the unimplemented primitive.
