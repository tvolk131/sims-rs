# An action whose entry runs at once is checked and run there and then

*Decided 2026-11-15.*

**Unverified.** Queueing an action whose interaction has flag 4 (0x45d20b,
the branch at 0x45d240) no longer fails. When the person queues it on
themselves, its check runs (0x458319). If the check passes and no action
is already running at once (the pointer at 0x6e5294 is clear), the action
tree runs straight away. It runs on the target object, with the target as
stack object, and takes seven parameters: the four arguments, the two
halves of the priority, and the tree. While it runs, the person's priority
(person data word 33) is the action's. Afterwards the priority is put back.
An action with no number then takes the next one. The person's temps are
restored if the check saved them (0x4582dd). Nothing is queued, and a
failed check, or a nested run, returns no number.

Two cases still fail loudly, because the original does something the port
does not model:

- an object other than the person queues such an action on the person;
- an action is queued on the person whose action runs at once. The
  original copies it over the running action (0x45d5a5).

A fault inside the tree becomes Unsupported. Every pick on Goth's person
16 now gets past this point and stops at Run Tree By Name mode 0 on
another object (0x44ba26). Reopen when the harness shows a pick that runs
at once.
