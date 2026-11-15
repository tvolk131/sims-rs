# Snap's modes 0, 1 and 2

*Decided 2026-11-15.*

UserData2's house 9 stopped at tick 232 on Snap in mode 0. The original
(0x454a4b) reads word 0 as the number of one of my frame's parameters:
from 0 to the frame's count of parameters (frame byte +7) less one,
compared signed, or native error 8. It reads the value through 0x47518b,
which is the start of the parameters (`lea eax, [ecx + 0x10]`); the
locals start after them (0x47518f adds byte +7 words), so it is not a
local. The parameter's value is the slot's number, and from there
it is mode 3: the stack object's routing slot table (+0x13c), the same
range check (native error 0x1b), the same placement (0x454ac5 on). The
slot's own record and flags are looked up by that number too.

With it, and the animator restore of the open branch, the house runs to
tick 241 and stops on object 140's tree 0x1003: objects 140 and 75 are
both not solid on one tile, and the test that follows (0x4358c3,
0x45728d) was not ported (the open branch `prim/both-not-solid` ports
it). (Read as a local, as a first version did, the
same Snap took another slot and the house ran on to tick 451; that was
wrong.)

Mode 1 (0x454a1d) puts me into the stack object's container slot 0 and
mode 2 (0x4549d1) one tile in front of it (its position plus 0x42228a's
step in its facing, the step's first word added to +0xf4, our second
coordinate, as for Drop). Both name no slot, take my own facing, and
always run the place's checks, as the byte the slot path sets from slot
flag 0x800 is 0 (0x454a23, 0x4549d7). Mode 1 skips the test that the
object has the container slot (0x454b19), so an object with none would
go on to a place in a slot it lacks; that fails loudly.

With the three modes, the animator restore and the test of two objects
that are not solid on one tile (the open branches), the house runs to
tick 875, where person 195's animator names a clip with its word 64
set (0x59db44), which is not ported.
