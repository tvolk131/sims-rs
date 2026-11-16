# Two objects that are not solid on one tile clash only against the same wall

*Decided 2026-11-15.*

UserData2's house 9 stopped at tick 241 (with Snap mode 0 and the
animator restore of the open branches): object 140 was placed on the
tile of object 75, both not solid (data word 8 flag 4), and the test
that follows (0x433aa6) was not ported. It is now (0x433a7f to
0x433b13):

- Each object's wall side (0x4358c3): data word 13 & 5 of 1 is wall bit
  2, of 4 wall bit 8, anything else none; then turned by data word 1
  halved toward zero through the turn table at 0x6f4670, which 0x53ba5f
  fills by shifting an edge bit up once per turn, 16 wrapping to 1. A
  half outside 0 to 3 would read past the table's four turns and fails.
- Each object's definition word 69 (+0x8a) of the definition it stands
  for (0x45728d): its own, or for a tile of a group (word 10 not 0) the
  first definition of its file of the same group with word 11 -1
  (0x43ae0a, 0x43adf7), the rule `Program::interactions` already used;
  it is now `Program::master_definition`. A group with no master fails,
  as the original reads through a null pointer there.
- If the word is 2 in one and not the other, no clash. Otherwise the
  two clash, with placement error 0x16 (`Refusal::SameWall`), when the
  sides are equal and not none.

0x16 is new to the place test's callers: Snap and the routing slots
treat it as any code other than 1, 7 and 11, and a group keeps an
earlier tile's code over it (0x422ba9 replaces only with 0x1e or 0x1f).
An object with wall rules is still refused before this test, except in
a container, so for now only the other object's side can be other than
none, and the two only share the tile.

The door test when an object is placed or taken off (0x433427,
0x4332b7) reads word 69 of the same definition the object stands for,
not its own, and now does too (`master_words`). A tile of a group whose
own word 69 differs from its master's would otherwise mark or clear its
tile's door flag wrongly. No house run changed.

With it the house runs to tick 641, where object 195 (tree 0x1009 node
11) runs Snap in mode 1, which is not ported.
