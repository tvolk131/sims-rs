# Person word 1 goes to the interface and is cleared

*Decided 2026-11-15.*

**Unverified.** No capture has a nonzero person word 1.

The person's tick hands a nonzero person word 1 to 0x4a69aa and then
clears the word (0x45ce61 to 0x45ce7e). 0x4a69aa shows the value over
the person, in one of three ways:

- Below -20000 it shows -20000 less the value as text (0x4d356a).
- From -20000 up to -10000 it plays the sound "sting_char_winner" for
  the person (0x57a54c) and shows -10000 less the value (0x4d3516).
- From -10000 up it shows the value as money (0x4d34f7).

Each is shown only while the interface's object ([0x6f2a58]+0x84) is
there. Showing means drawing text into a bitmap over the person
(0x4d322c), which is presentation. So the tick now only clears the word.
The middle range still fails loudly. Its sound could be a
`SoundEvent::Play`, but nothing shows whether the interface object
exists in a run with no view, and the snack order does not reach it.

The play-mode tests that set word 1 to 1 to get a fault outside the
thread now set -15000.

**On Goth.** In the snack order, Mortimer's word 1 holds 5 after step
820's scripts, the amount he just spent. That step now finishes (tick
89264). His carry clip
reaches its end two steps later and stops. At step 836 (tick 89279),
walking with the snack, his animator turns by less than a step
(0x58dfc0), which is not ported. Person 259 stopped at the same place,
at step 1104, while Mortimer was parked. That path decides between a step and a walk by distance
(0x58dfc0, 0x58f312) and picks a step clip by heading (the jump table at
0x58fad6), so it is not small. Strict mode on the Goth house (1102
ticks) and the K=1000 pins are unchanged.
