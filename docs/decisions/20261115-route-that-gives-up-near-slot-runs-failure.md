# A route that gives up near its slot runs the failure feedback tree

*Decided 2026-11-15.*

Go To Routing Slot and Go To Relative Position share the route steps of
0x45ad63. Step 16 (0x45aef7), giving up, sets the context's result to 1
if it was 0, writes it to person word 62 and goes to step 7, which fails
the primitive. Before that, when the slot says the route may end near it
(flag 0x4000), it looks up the tree named `Route failure feedback`
(0x6c457c) in the file of the frame's stack object (0x4021f2). If there
is one, it pushes it through the person's vtable +0x34 (0x45e59b), with
the stack object as an argument:
the person is woken (0x44574b), the tree runs on the stack object with
the arguments result, the context's word +0x8c, 0 and 0 (0x47484a), and
the person starts using the stack object (a use record at +0xb00 and the
object's +0xc6 counted, `count_use`). The step then returns 3, clears
person word 72, saves the context (0x45b9a0) and goes on in step 17
(0x45afb7), which writes the result to word 62 again and goes to step 7.
The tree's return value is not read. With no such tree the route goes
to step 7 at once, as before.

- A push that fails, where the original goes on to step 7 (0x45afb2),
  is not ported and fails loudly; so does a feedback tree with more
  than four arguments, since 0x47484a would read past the four given.
  Step 17, word 72 and the context are written only after the push, as
  in the original (0x45afac, then 0x45b6ba and 0x45b9a0), so a push that
  fails leaves the route as it was.

- The frame's stack object must exist: 0x45af5b reads its definition
  without a check, so a missing one fails loudly here.
- On the Newbie house (House07) the step runs at tick 1473: person 105
  gives up on a slot of object 64 with result 3, and the global tree
  0x18e returns in the same update. Strict mode now runs to tick 1802.
- The Newbie house matches track 2's recording through update 935 in
  everything the comparison reads (random streams, cursor, serial,
  globals, object IDs, and each object's data, temps, attributes,
  frames and person data), except the words already held as unknown:
  person word 37, which the planner copies from memory the original
  never set (see the planner entry), and locals that start on half of a
  pointer in the frame arena (2625 is 0x0a41, the high half of a code
  pointer of the capture). Global 12 differs at updates 1 and 3, where
  the original's interface waits on its tutorial dialogs.
- Reopen if a recording shows the feedback tree's push failing, or a
  route giving up near a slot without running it.
