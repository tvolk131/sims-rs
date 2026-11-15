# A blocked step, and K = 466

*Decided 2026-11-14.*

Tick 437 of the Goth house stopped where person 16's walk found its
next step blocked (`0x58f682`). Porting that led through two more
route steps in tick 438.

- **A blocked step ends the route blocked.** The walk checks that the
  person fits where the step ends (virtual function 0x54). If not
  (`0x58eecb`), it writes 3 to the route's end (+0x188; `esi` holds 3
  from `0x58edeb`) and goes straight to the update's tail (`0x58f688`).
  The step (+0x294, +0x298) is already written; the person and the
  animator's position stay where they were. That holds for the end of
  a path's extra step too. The stand's earlier writes stay (the
  destination's first two coordinates, the near-end and finishing
  flags, the state and the progress). Only the final copy of the whole
  position into the destination is skipped, and the state does not run
  again, so the standing animator never turns the 3 into 2. The capture's animator holds 3 after tick
  437.
- **Step 10 tries again.** A route whose animator stopped short of the
  path's end (0x589d47 gives 0, as it does for an end of 3) stops the
  animator (0x589e2d). If that fails, the run ends. Otherwise the next
  step is 6 (`0x45b371`). This was ported as a loud failure before.
- **Step 16 reads the context's slot, not a null one.** The route
  steps function reuses its slot argument (`[ebp+0xc]`): at `0x45aec6`
  every run sets it to the context's copy of the slot record
  (0x5c354f returns context +0xc). Giving up reads that copy's flag
  0x4000. The earlier entry's "null slot record on a later run" was
  wrong, and the loud failure is gone. A slot the route may end near
  still fails loudly (the failure feedback tree, `0x45af2a`).
- **Tick 438.** Person 16's route stops short, re-plans and finds no
  way, finds nobody in the way, and gives up. Word 62 takes the
  context's result, 13, left by the planning. The walk then goes on to
  door 87. The capture holds the same frames and word 62.
- **K = 466.** Ticks 437 to 466 equal the capture, but for the known
  last-place head fields. Tick 467 stops at Get Direction To (opcode
  12), not ported yet.

Reopen if a capture shows a blocked step that moves the person, or a
step-16 slot that differs from the context's copy.
