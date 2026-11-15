# Four types keep more saved attributes than their constructor allocates

*Decided 2026-11-15.*

A house read used to fail when a saved object held more attributes than
its constructor allocates (the larger of the definition's count and 8).
UserData's houses 21, 53 and 58, and most of the template houses of
UserData2 to 8, stopped there.

The object read (vtable 0x7c, 0x4342ae) reads the saved count from house
version 0x18. When the count is larger than the array and the read's
third argument is set, it frees the array, allocates one of the saved
length, zeroes it and reads the attributes into it (0x434345). Otherwise
it reads them past the end of the array (0x43437b). The house read
(0x43fe0a) sets that argument only for an object whose definition is one
of four types (0x43df98, 0x43dfa3, 0x43dfae, 0x43dfb9, compared at
0x4400a7):

- 0x3C08965F, the `*Placeholder` object;
- 0x7FD96B54;
- 0x4A70DF92 and 0x7BEA0977, only when the configuration's expansions
  byte (`[0x6e48bc]+0x66`) has Unleashed's bit (0x20). Without that bit
  both slots hold the first type (0x43fe9d).

The port takes the expansions from global 20, which the load sets from
the same mask (0xff in every boot here). An object of any other type
with too many attributes still fails loudly, and the message now names
its type.

**Unverified:** no capture holds a house with such an object.

What the houses do now (`sims boot`, 1000 ticks, answers 3): no surveyed
house stops at the attributes. Most stop at the household selection (no
family). The community lots, UserData's houses 40 to 48, stop where
Create New Object Instance mode 6 makes a person (0x44e60e).

Reopen if a capture shows an object of another type with more saved
attributes than its array holds.
