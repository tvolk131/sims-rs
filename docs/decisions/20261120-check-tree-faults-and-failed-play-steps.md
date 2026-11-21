# A check tree's fault keeps its trap; a failed play step hands back what it caught

*Decided 2026-11-20.*

## Pie listings on the Goth house

The roadmap's integration section listed check trees that failed on the
Goth house: Manage Inventory operation 4 and scope 12. Both are ported
already:

- Manage Inventory operation 4, which walks the inventory, in #173;
- scope 12, the check tree's advertisement range, in #45. #146 kept the
  advertisement record Find Best Interaction last copied, and #173 made
  the record readable before the first copy, which is what the Goth
  house's listings needed.

A scratch probe on current main listed every person's pie menu on every
object of the Goth house, after 0, 1500 and 6000 ticks. Neither of those
two fails any more. The listings that fail today are these:

- **Call Cab's entry check.** Objects 4, 10 and 253 offer Call Cab (as
  #173 found); the probe's case is object 10's check tree 0x2074. The
  check formats the entry's name with `$$Local:1` tokens, which the
  formatter (0x44bf1a) does not port. It fails at every tick probed.
- **Joining a running interaction (0x45866d, 0x41ee99).** From 1500
  ticks. Listing a pie menu offers to join the interaction the listing
  person is running, when it can be joined, and that offer is not ported
  (`orders.rs`, after the entries are listed). The offer is made on
  another person's pie menu. So a person running a joinable interaction
  meets it when listing any other person, an idle one included. Listing
  an object, or itself, meets it too when that menu offers nothing and
  the listing falls back to another person on the same tile
  (`list_with_tile`).
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

- an entry's check while a pie menu is listed, an advertisement is
  weighed, or a queued action is checked before it starts (`autonomy`);
- the check Find Best Object for Function runs (`routine` names it).

A check tree's fault can also pass through other work on its way out:
the Init of an object a check creates, another object's callback, a
route, and so on. Those used to turn the fault they met into unsupported
text, "what the work was: the fault". For a check tree's fault they now
keep it whole, in `Trap::Within { context, fault }`, with the same text
(`Fault::unsupported_in`, `Fault::nested`). Faults of any other kind are
turned into text as before.

`Trap::unported` says whether a trap is from something not ported. For
`CheckTree` and `Within` it asks the inner fault. The window falls back
to listing every entry only when that is true.

What changes:

- The text no longer starts with "unsupported: ".
- A check tree's fault met in another object's work (`Fault::nested`)
  now reads "object …, tree … node …: check tree …". Before, it read
  "… (in object …, tree … node …)".
- A check that fails inside other work now reads as failed, as it does
  when met directly. Before, it read as unported, and the window listed
  every entry:
  - inside a check tree that Find Best Object for Function runs from a
    menu check;
  - inside an object a menu check creates, whose Init runs Find Best
    Object for Function.
- An `Unsupported` trap whose own text happens to start with "check
  tree" is now unported. The old parser took it for a failure.
- A check tree's fault met in a route's callbacks on the routing
  person's own thread is now held in `Trap::Within`, which reads as the
  check does. Before, only its trap was kept. Either way the fault names
  the check's frame, the top of that thread's stack, and not the
  callback that ran it; the animator's arrival check keeps only the trap.
  The callback's place is left out of these diagnostics.
- `sims boot --play` counts the errors, skipped frames and sounds a
  failed tick hands back with the rest.
- A check tree's fault met while a tree is pushed (Gosub Found Action,
  Idle For Input, a door's tree) is still turned into text. Pushing a
  tree runs no check, so none is expected there.

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

Each skipped frame is a `SkippedFrame` that says which of the lot's
ticks it follows, counted from the load (`Lot::ticks`). 0 means the
frame before the first tick. The window used to guess which frame was
which from whether the step was the first. Now it orders them by tick,
and its lines say which frame it was: "the frame before the first tick
was skipped: …" and "the frame after the tick was skipped: …". The lines
leave out the tick for two reasons:

- an object's line starts with the clock's tick count, which is another
  count;
- the error log takes the same failure at later ticks for a repeat.

The window logs the errors a failed step hands back before it shows why
the lot stopped.

A failure that is a dialog waiting for the player (`StepError::Asking`)
is unchanged: the lot is put back as it was before the tick, so nothing
caught in it stands.
