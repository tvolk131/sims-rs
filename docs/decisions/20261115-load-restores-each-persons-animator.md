# The load restores each person's animator: its saved layers pose the skin, and its head keeps its angles

*Decided 2026-11-15.*

The Pleasants' first frame stopped: person 242's head follows person
241, whose animator was never ticked or drawn, so the point on top of
241's head was what the load left in its skin. The load's restore of the
animator (0x5918ba, `crate::restore` in `sims-vm`) is now ported as far
as the simulation reads it:

- **The layers.** Each of the three saved descriptors (foreground, carry
  slot, base slot; "%s;%d;%d;%d;%d;%d;%d;%d", written at 0x5864ff) is
  read as 0x591eec does: the clip's name up to the first ';', then the
  priority, the speed, the phase and the weight (each in thousandths,
  0x666a24), the end mode and two words nothing ported reads. The C
  runtime's `atoi` (0x636029) is ported with its wrap-around. A
  descriptor that runs out of ';' makes no layer. Each layer is made at
  the load's clock and inserted in the skin by priority (0x5a0a5b,
  0x599128), then the skin is updated with them at once (0x5992b3,
  forced) and placed on the person's tile (0x592132).
- **What stays.** The person's reset then takes the layers out of the
  skin (0x589220) but not the bones they posed: the skin holds them
  until it samples a layer again. That pose is kept in
  `World::held_poses` (snapshot format 28): the layers and the
  placement. The head's measurement samples them under whatever plays
  now, and a person not placed since the load is placed where the load
  put it. The skin's first update with a layer drops them, and the
  bones they moved count as moved (`World::translated_bones`).
- **The head.** The heading, the target's heading, the turn and the
  target point are restored (a measurement whose target object is gone
  reads the point, 0x5906bc). The load's head step sees the
  constructor's zero data words (the person's data is read after its
  animator), so the measurement does not run. The constructor zeroed
  the head clock (+0x164) and set the animator's clock to the clock at
  the load's tick count (0x585483), so the step runs unless that clock
  is 0 (0x590788): the turn relaxes once (x0.8) and settles. The events seen (+0xa4) are kept; the
  reset clears +0xa0.

**Only after a reset.** Everything above assumes the person is reset
at step 13 of the load, which happens only when its type's scripts
changed since the save (`World::changed_types`, 0x459bf6). On the Goth
and Newbie houses every person's type changed; on the Pleasants none
did, so their people keep what the restore read: the foreground clip
plays on from its saved phase (person 242's fidget ends at update 13 in
the capture), the person stays in its saved routing state (0, 4 or 9)
and its saved route contexts stay. That live restore is not ported, so
a person the load does not reset fails loudly.

**Fails loudly:** a person the load does not reset, a saved placement
compensation other than 0 (+0x8c, which the placement adds) and a
descriptor whose clip our content does not hold (the original makes no
layer for a name its table lacks, which is not ruled out).

**Against the capture** (`house06-pleasants-ticks-20261115`, entry 0,
the frame before tick 1), measured with the reset assumed for the
Pleasants' people, before the check above: person 242's heading, target heading and turn
are bit for bit the original's, and so are two of the three coordinates
of its target, the top of person 241's head. The third, the height, is
one unit in the last place off (0x4187338d here, 0x4187338e there); the
cause is not found. Nothing reads the target before the next
measurement writes it. Persons 241 and 243 keep their saved angles, as
in the capture.

**Strict runs.** The Goth and Newbie states are unchanged but for the
new field and the kept head targets: by tick 33 every Goth's skin has
sampled a layer, and the pins' format-27 encodings equal the old hashes
once the targets no measurement has written since the load are cleared.
Those targets are what the save held (each Goth's is 0x00fdf000 three
times, memory the original never set before saving), so the tick-33 pin
moves; from tick 1103 on, the Goth pins' format-27 encodings are the
old hashes, through tick 9906 (c8240fc7…, where strict mode stops), and
only the new field (empty by then) moves the format-28 hashes. The
Newbie at ticks 1802 and 4052 is the same. The Pleasants now stop at the load, on person 117, the
first person whose type did not change. UserData2's house 9 (whose
people are all reset) gets past its first frame and stops at tick 232
on Snap mode 0 (0x4549c7).
