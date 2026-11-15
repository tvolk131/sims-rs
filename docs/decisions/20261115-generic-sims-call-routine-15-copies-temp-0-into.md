# Generic Sims Call routine 15 copies temp 0 into person word 72; the footprint is worked out after word 73 is cleared

*Decided 2026-11-15.*

**Unverified.**

Routine 15 (0x44fe3c; the Generic Sims Call jump table at 0x45143a)
casts the running object to a person (0x635abe; a failed cast throws,
native error 0x1c). It copies temp 0 (object +0x3a, the word routine 28
reads its lot from) into person data word 72 (+0x61c) and clears word 73
(+0x61e). When temp 0 is 0 it then works out the person's footprint
again where it stands (0x433d16 with +0xf4 and +0x100, its width through
0x43519f). That reuses the footprint that placement already ports. Word
73 is cleared first, so the footprint never takes the even-facing
extension from it. When temp 0 is set the footprint is kept, whatever
word 73 held before.

The port works the footprint out before it writes anything, from the
person's words as they will be, so a footprint that fails loudly (a
facing outside 0 to 7) leaves the person as it was. The original cannot
fail there.

The Goth house's social pick runs it once, at tick 48, on person 21 with
temp 0 at 8, so only the copy and the clear are reached. The footprint
branch is checked only by the synthetic tests. No capture covers the
pick. The social now runs to tick 261, where it stops on scope 24 field
9 (a neighbor's family's friend count, through the family friend-count
cache). That needs state the World does not hold yet. Reopen if a
capture of the pick disagrees.
