# A turn of less than a step is made at once; Run Functional Tree's flag names the action's icon; Find Location For mode 1

*Decided 2026-11-15.*

**Unverified.** No capture covers any of the three. In the Goth captures
(updates 1 to 1000) no person ends an update in state 3 short of its
destination's direction, and no direction moves by less than a step
without a turn clip.

**The small turn.** State 3 compares the turn d with the step (a quarter
over the speed) and then the distance to the destination (0x58dfc0). An
animator that has arrived is 0 away (0x58df1a). Below 0.5 (0x6d07e4) the
animator is put at its destination at once (0x58dfcf): the position
takes the destination's two coordinates, the direction the destination's
(virtual function 0xd0, 0x58488b), and data word 1 the direction
(0x5920e8). Then state 3 runs again (0x58f089), which finds it arrived and
aligned. No clip plays. Global 0x6f6d64 becomes 4, which is not kept.

The step clip by heading (0x58f312, the jump table at 0x58fad6) and the
clip turn with a distance (0x58f4ee) come only after an animator that has
not arrived. From 0.5 up to 3.1 (0x6d07b0) away it takes the step, and
from 3.1 up it turns with a clip. State 3 short of its destination still
fails at 0x58df1e, so neither is reachable and neither is ported.

**Run Functional Tree's flag.** The person's virtual function 0x34
(0x45e59b) takes a flag as its fourth argument. With it set, the object
becomes person +0xa54, the icon of the running action at +0xa48
(0x45e5d4). The use entry keeps the flag (entry +8). When a use ends with
its frame (0x45e64c), each entry with the flag, first to last, writes its
object there again (0x45e65d to 0x45e665), so the last one's stays. An
entry ended by a shallower return writes nothing (0x45e678). Of the
ported callers only Run Functional Tree passes a flag. Run Tree By Name,
Idle For Input (0x45dca2), Gosub Found Action (0x45a920) and the route's
pushes pass 0.

**Find Location For mode 1** (0x44b519 to 0x44b565) takes the stack object
off the lot: its virtual function 0x50 with 1, as the removal queue
calls it, then the true branch. A tile of a group (definition +0x14 set)
is first tested with 0x42344e and may go to 0x42346a. That is not
decoded, so it fails loudly, as Remove Object Instance does.

**On Goth.** In the snack order Mortimer's animator is 5.7e-6 short of its
direction at step 836 (tick 89279) and faces it at once. At step 1035 his
tree 0x1050 runs object 121's function with flag 1, and object 121
becomes his action's icon. At step 1088 his tree 0x1002 takes the snack
(object 263, in his hand) off the lot. Person 259 makes a small turn at
step 1103 (-0.025). At step 1104 (tick 89547) it removes itself (Remove
Object Instance in tree 0x0176), and taking a person off the lot (the
person's virtual function 0x50) is not ported. That is big, so the order stops
there.

Strict mode on the Goth house gives the same output for 1102 ticks. It
now runs through tick 1103 (person 259's small turn) and stops at tick
1104 on the same removal. The K=1000 pins are unchanged.
