# Alter Budget category 8 writes into the neighborhood request queue pointer

*Decided 2026-11-18.*

**Unverified.** No run of the original was recorded with a nonzero
category 8 spend. What the moved pointer would address depends on the
original process's heap at the time of the next access.

Alter Budget accepts category 8 (0x44eb10). When it spends outside budget
mode 1, the helper subtracts the amount from funds (0x46d92d), then adds
it to the word at simulator +0x184 plus four times the category
(0x46d933). Both operations wrap in 32 bits. Category 8 reaches +0x1a4.
The constructor stores the address of a newly allocated 28-byte sentinel
there and makes its next and previous links point to itself (0x46ce0c to
0x46ce1e). After the addition the helper updates the funds globals and
sends signal 0xda (0x46d960 to 0x46d96d).

The SIMI reader and writer pass +0x184 to the day routine (0x46d1e4).
The routine transfers eight words for a nonzero flag or seven for a zero
flag (0x46d270). The eight words end at +0x1a0. The other five days
start at +0xe4. Midnight (0x46d430 to 0x46d45e) and the old-save reset
(0x46d97a) also copy eight words for today. This save path never transfers
the queue pointer.

Generic Sims Call routines 39 and 40 enqueue neighborhood requests and
later look up and remove completed requests (0x451348, 0x451367). The
enqueue uses the sentinel's previous link (0x46dcc2), and the lookup
reads its next link (0x46db59 to 0x46db63). The result goes back into
the object's temps (0x451378 to 0x451397). After a lot update, unless the
idle call exits early, it runs the queue (0x46da90 to 0x46daa0).
That pass dereferences the pointer before checking for an empty queue
(0x46dbf0 to 0x46dbfa), calls the neighborhood services (0x42db51,
0x42debe), and ages out requests (0x46dbab). The simulator destructor
walks the list and frees its sentinel (0x46cf89, 0x46dcea, 0x46dd1e).

Keep the category 8 trap in `spend` from
[the Alter Budget decision](20261115-alter-budget-spends-into-todays-budget-entry.md).
A budget field could record how far the pointer moved, but could not
reproduce the next dereference. The current VM has neither the native
address space nor the neighborhood request service. The write, its
consumers and any state they would require are outside this port. Reopen
with a targeted native trace of a nonzero category 8 spend through the
next queue access and a model for that access.
