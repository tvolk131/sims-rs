# Find 5 Worst Motives sorts with the C runtime's qsort

*Decided 2026-11-15.*

Find 5 Worst Motives (primitive 33) is the walk-by portal's: its "show
leave dialog" sorts the person's own motives (all operands 0). The
census counts two uses in the base game. It is now ported
(`primitives/worst_motives.rs`) from the general table (0x45246a), which
sends opcode 33 to 0x44d077.

- **Whose.** Operand word 2 at 1 names the stack object, at 2 no one,
  and otherwise the running object. No object there, or one that is not
  a person (definition word 9 is not 2), throws native error 0xa.
- **Operand word 3** must be 0 to 2 (native error 0x1e otherwise). The
  original reads it for nothing else.
- **The sort.** The motives 5, 6, 7, 8, 9, 13, 14 and 15, in that order,
  are sorted by the C runtime's `qsort` (0x6356ba). Eight elements are
  few enough for its selection sort (0x63580e), which is not stable. The
  comparison (0x44d03b) treats two motives less than a hundredth apart
  as equal, so near ties come out in the selection sort's order.
  Motive 5, for one, ends last when all eight are equal.
- **The temps.** The first five motive numbers, the lowest first, become
  the running object's temps 0 to 4 (+0x3a), whoever's motives they are.
  The branch is true.
- **Shared sort.** Find Best Interaction's port of the same `qsort` is
  now generic (`autonomy::qsort`) and serves both. Its order is
  unchanged.
- **Precision.** The difference is rounded to single precision before
  it is compared, as Find Best Interaction's comparison already takes
  the FPU to run (`autonomy.rs`). That matters only for a difference
  within rounding of a hundredth.

**Left out.** The original keeps a pointer to the person's motives in a
global (0x6e50a4) for the comparison. Nothing else reads it.

**Unverified.** No capture covers the walk-by portal's leave dialog.
The tests are synthetic.

**Strict runs.** The Goth and Newbie strict runs are unchanged. Neither
reaches this primitive, or it would have stopped on the unimplemented
primitive.
