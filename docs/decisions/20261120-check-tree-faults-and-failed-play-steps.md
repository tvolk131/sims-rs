# A check tree's fault keeps its trap; a failed play step hands back what it caught

*Decided 2026-11-20.*

## Pie listings on the Goth house

The roadmap's integration section listed check trees that failed on the
Goth house: Manage Inventory operation 4 and scope 12. Both are ported
already:

- Manage Inventory operation 4, the advertisement record's start, in #173;
- scope 12, the check tree's advertisement range, in #138 and #146.

A scratch probe on current main listed every person's pie menu on every
object of the Goth house, after 0, 1500 and 6000 ticks. Neither of those
two fails any more. The listings that fail today are these:

- **Call Cab's entry check (object 10, check tree 0x2074).** The check
  formats the entry's name with `$$Local:1` tokens, which the formatter
  (0x44bf1a) does not port. It fails at every tick probed.
- **Joining a running interaction (0x45866d, 0x41ee99).** From 1500
  ticks, offering to join an interaction another person is running is not
  ported.
- **A visitor's chosen action on a category 4 or 7 object (0x41f660).**
  From 6000 ticks, check trees 0x2025 and 0x103e ask the relationship
  between the visitor and the person ordered, which is not ported.

These are new needs; this change ports none of them.

## A check tree's fault keeps its trap

A check tree's fault used to become `Trap::Unsupported` text, "check tree
… of object …: …". The window (sims-game) parsed that text to tell a
check that met something not ported from one that failed. That made a
fault inside a check read as merely unported once it was wrapped.

Now the fault is `Trap::CheckTree { routine, tree, object, fault }`,
whose inner fault keeps its own trap. The wrappers are:

- an entry's check while a pie menu is listed, or an advertisement is
  weighed (`autonomy`);
- the check Find Best Object for Function runs (`routine` names it).

`Trap::unported` says whether a trap is from something not ported. For a
check tree it asks the inner fault. The window falls back to listing
every entry only when that is true.

Two differences from before:

- The text no longer starts with "unsupported: ". The rest reads as
  before.
- An `Unsupported` trap whose own text happens to start with "check
  tree" is now unported. The old parser took it for a failure.

Nothing in the interpreter branches on the trap's kind, so strict runs
are unchanged. The House05 boots to 1102, 3602, 9906 and 43200 ticks keep
their hashes.

## A failed play step

In play mode a step whose lot update fails used to drop what the update
had done before the failure:

- the errors caught in the objects' ticks;
- the frame before the first tick, if it was skipped;
- the sounds.

`StepError::Tick` now carries them in `update`. That field is None in
strict mode. The update is kept by `Interpreter::lot_update_kept`, which
`lot_update_with` wraps unchanged.

Each skipped frame is a `SkippedFrame` that says which tick it follows.
Tick 0 means the frame before the first tick. The window used to guess
which frame was which from whether the step was the first. Now its lines
say so: "tick 1, the frame before it was skipped: …" and "tick N, the
frame after it was skipped: …". Starting with the tick lets the error
log take the same failure at later ticks for a repeat, as it does for an
object's error. The window logs the errors a failed step hands back
before it shows why the lot stopped.

A failure that is a dialog waiting for the player (`StepError::Asking`)
is unchanged: the lot is put back as it was before the tick, so nothing
caught in it stands.
