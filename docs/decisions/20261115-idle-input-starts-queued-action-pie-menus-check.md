# Idle For Input starts a queued action; the pie menu's check

*Decided 2026-11-15.*

Idle For Input, asked to and interrupted with something queued, now
starts the front action (0x45dc14 to 0x45dd1f) instead of failing. The
action is taken off the queue and checked as the pie menu checks it
(`Machine::check_chosen`: 0x458319 on a context made with byte 0 and no
advertisements). That is autonomy's check without its data word 25
rule, with the check tree's arguments 0, 0, 0, 0, and the result left in
the action's flags 4, 8 and 0x10. A passing action's tree is pushed as
Gosub Found Action pushes one, with the action's own arguments, and it
becomes the running action. The frame's primitive state becomes 1, so
when the tree returns the instruction runs again and starts only an
action pushed to the front. A refused action is cancelled on its target
(function 4) and the thread yields. The person's temps, saved before
the check tree, are put back last.

Ported from the original's code alone; no capture reaches it yet, so it
is **unverified**. Two assumptions: a queued action is taken not to join
another (the save does not keep its word +0, which the pie menu's
actions have as 0), and the interface flag 0x6e5134, which would pass
an action whatever its check tree returned, is taken to be clear, as
autonomy already takes it. A tree that cannot be pushed, where the
original cancels the action, fails loudly. Reopen with a recording of a
player's order (goal step 4).

A check tree the file lacks now writes the synchronous run's mark above
the top frame before it fails (0x4749d9), in this check and in
autonomy's, as `Interpreter::callback` already did; its words can
become a later frame's locals. The Goth house's first 1000 ticks are
unchanged by it.
