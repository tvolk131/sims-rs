# Snap mode 4 snaps to one of the lot's global routing slots; an action pushed to the front goes first

*Decided 2026-11-15.*

**Unverified.** Snap (opcode 46) mode 4 (0x454ab6) names its slot by
word 0 in the lot's global routing slots, the table Go To Routing Slot's
source 2 reads (the stack object's lot, +0xdc, at +0x9c: the kind 3
entries of the global file's SLOT 100). From 0x454ac5 on it is mode 3's
code, so the place is still the stack object's and every later step is
the same. The flags are the lot's one copy, kept under object 0 as Go To
Routing Slot keeps them. An index outside the table, a negative one
included, is native error 0x1b. Modes 0 to 2 still fail loudly.

Queueing an action with flag 2 (0x45d322 to 0x45d47f) no longer fails.
The action has already taken its number. With flag 0x40, the person's
running action (+0xa48) loses its flag 0x80 (0x41f167), and the new
action takes that action's name (0x41f1ae). Taking the name sets the new
action's flag 0x80, which is ported; the name itself is not, since the
queue does not keep names. A queue
of the routine's own takes the new action and then the queued ones in
order, and the person's queue is given them back. A full queue passes
only its first seven, and only seven are given back, so the new action
goes in with the first six and the last two are dropped, with no object
told. The action is always queued, and the branch never looks at an
action running at once (0x45d5a5), so a push to the front from inside one
is queued too.

In play mode, after the Have A Snack order (50 steps, then the order),
Mortimer (object 21) snaps to himself with global slot 5 at step 820
(tree 0x0127 node 2). Its flags are 0x1010, so he turns by 4 where he
stands, from facing 6 to 2. In the same tick the fridge's tree 0x1018
(node 10) pushes his action 4170 on the snack (object 263) to the front
of his empty queue. He now stops in that tick at Alter Budget spending 5
in category 3 (0x46d91c, tree 0x1007 node 2). Strict mode on the Goth
house is unchanged. Reopen when a recording of the snack order disagrees,
or when a push to the front meets a full queue.
