# The head relaxes as the original's runs do, not as its code reads

*Decided 2026-11-14.*

With the follow off, the head's turn is multiplied by 0.8 each tick
(0x590886). The code image shows `fmul qword [0x672030]`, a double 0.8,
and nothing else writes the turn (only the step, its settle to 0, the
constructor and the snapshot). Yet in the fifth capture, every relax
step where the two products differ follows the product with 0.8 as a
float (0x3f4ccccd), rounded once: five steps of person 21's, ticks 814
to 833. The other 17 steps give the same result either way, and no step
in ticks 1 to 871 follows the double. Rounding up or a 53- or 24-bit
precision of the double product does not give the captured values.

The captures are how the original is observed, so `Head::relax`
multiplies by the float. Why the run differs from the code is
**unknown**; the captures run the game under an emulator, and its x87
may round its own way. Reopen if a run on real hardware follows the
double, or if a relax step in a later capture follows neither. The
persistent one-unit head target differences (see "Ticks 334 to 366" in
the roadmap) may have a cause of the same kind.

Also K = 871: a carry layer that moves no bone the head reads is passed
over, and Set Motive Change is ported from 0x45a16f.
