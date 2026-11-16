# The relationship panel leaves out whom the original leaves out, and lists stars

*Decided 2026-11-15.*

This replaces two points of
`20261115-relationship-panel-lists-who-sim-knows.md`, which left both
unported: the original's check of who may be listed, and its Superstar
button.

- **Who is left out** (0x4e9e9b..0x4e9ed8, in the list's walk over the
  neighbor records, 0x4e9deb). A record is listed only if its person is
  a neighborhood character of a family, or has person data word 32 at 6:
  - A neighborhood character is a type whose definition comes from the
    neighborhood's `Characters` folder: 0x423f30 tests bits 2 and 3 of
    the type's record (+0xb4) for 4, the kind that folder is registered
    with (`20261115-household-selection-works-out-again-who-visitor.md`).
    Here those are the GUIDs whose definition is from the `Characters`
    mount (`household::Acquaintances::characters`).
  - Of a family: person data word 61 is not 0.
  - The person data is read as 0x4ea662 reads it: the person's on the lot
    when one of the type is (0x4e9608, the first in the lot's people
    order), or else the type's first record (+0x74).
  - So the game's NPCs (the maid, the repairman, the salesman and the
    others, whose types are not the neighborhood's) are never listed,
    nor a character without a family, unless word 32 is 6. What a 6 in
    word 32 stands for is not known (**unverified**); the household
    selection gives it 0, 1 or the saved value.
- **The Superstar button** (0x51a7ea) can be pressed when the installed
  packs' mask (`[0x6e48bc]+0x66`) has Superstar's bit, 0x40; otherwise
  it is disabled. The Complete Collection has the pack, so it can be
  pressed there. The host passes the packs (`Install::expansions`) in
  `Acquaintances::read`, and `Snapshot::superstar` carries it to the
  layout.
- **The Superstar list** (mode 2 at 0x4e9f22) keeps the people whose
  person data word 81 is not 0, read as above. Superstar keeps fame
  there (`20261115-deselecting-sim-ported-but-animators-refresh.md`
  reads its star markers from it).
- **On the Goths** nothing is left out: Mortimer knows Bella and
  Cassandra, both characters of his family, and neither has fame. The
  panel's digest is pinned again for the Superstar button alone: drawn
  in its normal cell instead of its disabled one, 232 pixels in its 20 by
  20 square.

`with_relations` now takes the friendship threshold, the characters and
the Superstar flag together (`household::Acquaintances`).

**Unverified:** the look of the enabled Superstar button and of the
Superstar list (no screenshot); the meaning of word 32 at 6.

What would reopen it: a screenshot of the panel that lists someone this
leaves out, or the reverse.
