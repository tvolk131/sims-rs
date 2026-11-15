# A person the load does not reset plays on from its saved layers

*Decided 2026-11-15.*

Load step 13 resets a person (0x589220) only when its type's scripts
changed since the save (0x459bf6). The Goths' and the Newbies' types all
have, but none of the Pleasants' has, so the Pleasants stopped at the load
on person 117: the restore (0x5918ba) failed loudly for a person it does
not reset. That person now keeps what the restore read and the
constructor left (`Interpreter::restore_live`):

- **The constructor's fields** (0x587d..), without the reset's own
  writes, so +0x1a0 (`Animator::promote`) stays 0 where the reset sets 1.
  The animator's clock (+0xb4) is the clock at the load's tick count
  (0x585483, from 0x587f5a).
- **+0xa4 and +0xa0 as saved:** the events seen and whether Animate
  Sim's clip plays.
- **The routing state** (+0x190) as saved when it is 0 to 3, and 3
  otherwise (0x591d44 to 0x591d64). Person 117's saved 4 and person
  240's 9 become 3.
- **The three layers, live in their slots.** Each is made at the clock,
  then the restore sets its phase (+0x28) and end mode (+0x40). The
  skin's forced update (0x5992b3) then updates each, by priority, at no
  time gone by: current and previous are the clock, and the event time
  is the phase times the clip's length. The base layer waits in the base
  slot (`Base::Loaded`, its layer in `World::base_layers`). The
  constructor's posture is -1, so the first base update (0x590430) takes
  it out unless the person's posture word is -1 too.
- **The placement** (0x592132) sets the unset position and the
  destination from the person's tile. The head then takes its step as
  after a reset, and the head clock is the clock.
- **Nothing is held** (`World::held_poses`): that is for a skin whose
  layers the reset took away.

**What fails loudly:**
- A restored layer whose update at load reaches a timed event. That
  update comes before the person's data words are read, and what the
  event would do then is not ported.
- A saved base layer whose clip has timed events.
- As before, a clip our content lacks and a compensation other than 0.

No new field: `Base` gains a variant, and states without it encode as
before.

**Against the capture.** With this and the saved route contexts, the
Pleasants (`house06-pleasants-ticks-20261115`, answers 3, 2, 2) run 1000
updates. The random streams, the cursor, the action serial and the
globals match the capture at every update. The objects match too, apart
from the differences already recorded in
`20261115-household-selection-works-out-again-who-visitor.md`:
- person 241's frame tree bit 15 (0x2029 here, 0xa029 there);
- person 243's object data word 50 and last result (an unknown load
  tree), until updates 116 and 614;
- person word 37, which the original never set: it differs for person
  117 at updates 2 to 380 and for person 241 at 669 to 800.

The capture's animators at entry 0 agree: persons 117 and 240 are in
state 3 with the destination at the position, and persons 241 to 243 are
in state 0, playing, with the foreground layer's words matching.

**The Roomies** (House10, answers 3, then 2, then four 2s) now get past
the load and stop at tick 2 on object 176's tree 0x0176 node 26, which
asks about a water tile (0x482301, not ported).

**Strict runs.** The Goth and Newbie strict runs are unchanged: every
person there is reset.

Reopen if a capture shows a person the load does not reset whose saved
routing state above 3 is kept, or a base layer that outlives the first
base update.
