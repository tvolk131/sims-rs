# Run Tree By Name modes 0 and 1 on my thread; an action queued during an at-once run takes its place

*Decided 2026-11-15.*

**Unverified.**

**Run Tree By Name.** Mode 0 (0x44bb75) runs the named tree of the stack
object's type on the running object's own thread. I stay me, the stack
object stays its stack object, and my temps are the arguments. Mode 1
runs it on the stack object's thread, so mode 1 with myself as the stack
object is the same run. The tree runs at once, to its end, and its result
is the branch. 0x4749b5 treats the running thread like any other: it
pushes a stop frame and the tree's frame above my frames, runs until the
stop frame, and then puts back the thread word it cleared. The port runs
the tree above my frames in the same way.

**Queueing during an at-once run.** While a person's action runs at
once, an action queued on that person is copied over the running action,
including its number and flags (0x45d5a5, 0x41edbf). It is also queued,
unless the queue is full. The at-once run then returns the queued
action's number and takes none of its own. This replaces the loud
refusal recorded in [20261115-action-whose-entry-runs-once-checked-run-there](20261115-action-whose-entry-runs-once-checked-run-there.md).

With flag 0x40, the original also gives the queued action the running
action's name (0x41f167, 0x41f1ae), which sets its flag 0x80. The flag is
set here, but queued actions' names are not kept.

In modes 0 and 1 a check tree's advertisements (the global record at
0x6c437c) stay the same record through the named run.

**On Goth.** The first pick on person 16 now queues the social through
the object its tree made, and returns that action's number. The next
step stops on Animate Sim on the base slot (0x58957e). Reopen when the
harness records a social pick.
