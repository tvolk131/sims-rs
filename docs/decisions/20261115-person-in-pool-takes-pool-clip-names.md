# A person in a pool takes its clip names from the pool's tables

*Decided 2026-11-15.*

The name of one of a posture's clips (0x59db2f) failed loudly for a
person whose word 64 is set, which the animator's refresh sets for a
person on a pool tile (0x58da12). UserData2's house 9 stopped on it at
tick 875 (person 195, with the open Snap, clash and restore branches).
It is now ported (0x59db44 to 0x59dbbe):

- The global file's string tables at indices 10 to 12 of 0x6f7698 are
  STR# 158, 159 and 160 (0x59d974 to 0x59d9a1). For a person in a pool
  the lookup reads 158 plus bit 0 of word 65 for an adult (word 58 above
  17), or 160 for a child.
- That lookup replaces the type's own table (STR# 150 of its file)
  whatever it finds: a name ends the lookup, and no name or an empty one
  goes on to the global tables for age or sex (151 to 153), then 150.
  The type's own table is never used for such a person.

The lookup reads word 64 when it runs, as the port's other word-64 reads
on main do. With it, house 9 runs to tick 1022, where person 195 stops
at the end of its path in the pool (0x58e83b). That needs the walk's
flag +0x290 (set at 0x58edec, read at 0x58ef8e), which the animator does
not keep yet.
