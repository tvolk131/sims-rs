# Get Direction To, and K = 583

*Decided 2026-11-14.*

Tick 467 of the Goth house stopped at Get Direction To (opcode 12).

- **Ported from the original's code** (`0x452003`). The destination
  is resolved for writing first, as Random Number does. Then come the
  object to look from (my object data word 11, or an operand with
  flag 1) and the stack object, missing as native errors 0x17 and
  0x15. The result is the route slots' direction (`0x4520e9`), plus 8,
  modulo 8. Global 34 aliases the destination as in Random Number. A
  destination with no word behind it (a literal, or a motive) fails
  loudly, because the original writes through a null pointer there.
- **The positions go in the native order.** The original passes +0xf4
  first, which is our position's second coordinate, as the slot code
  already knew (`pos = [target[1], target[0]]`). A first port that
  passed ours in order matched every object for 117 more ticks but
  left the lot stream one draw behind from tick 467 on. With the wrong
  direction, person 21's tree 0x1017 found object 126 beside door 93,
  so tree 0x100a skipped its Random Number.
- **How it was found.** A new capture of the original with a write
  watch on the active random state (`0x6e54ac`, `--watch`) logged each
  draw with its stack. Tick 467 had one, from Random Number
  (`0x451edd`) in person 21's thread. Our trace showed the tree that
  draws it is reached only when 0x1017 returns false. The capture
  matched the earlier one tick for tick, which also shows the
  original's runs are repeatable here.
- **K = 583.** Ticks 467 to 583 equal the capture, the three random
  streams included, but for the known last-place head fields. Tick 584
  stops at Snap (opcode 46).
- **What would reopen it:** a Get Direction To with a destination or
  object scope the Goth run has not reached.
