# `open-pack` behavioral specs

Every object in `../pack.toml` has a spec here, `<object>.test.toml`, run by
`sims-antics test open-pack/tests` on a fresh `sims-vm` lot per case. The format is
documented in `docs/decisions/20261124-antics-test-runner.md`; `cargo test -p sims-antics`
runs every spec (`antics_test_runner`) and fails if an object has none
(`open_pack_behavioral::every_open_pack_object_has_a_behavioral_spec`).

Patterns the specs use, for writing new ones:

- **Guards at their boundary.** Test the last value that offers an entry and the first that
  does not (`bladder = 89` offered, `bladder = 90` not), and `occupied = 1` for an entry with
  `lock: self.occupied` (the lock adds an occupancy check to the guard).
- **`interaction` for what the player sees.** It goes through the real pie menu and the Sim's
  queue. `result = "unavailable"` says only that the Sim never ran the entry: the menu did not
  offer it, the queue did not take it, or the Sim dropped it. To pin a refusal on the menu, test
  the entry's `guard` too.
- **Room through the room impact, never `sim.motive.room`.** After each of the Sim's turns the
  person tick sets Room from the score of the room the Sim is in, so a script's write to Room is
  lost. An object changes Room by its room impact (`self.room_impact`, data word 7), which the
  room score adds up; the runner's objects stand in no room, so the specs check
  `data = { 7 = ... }`.
- **Cancel-safe loops through `action`, as the Sim, with a priority.** `cancel_on_abort`
  (and the `std::motives` loops) stop when the Sim's action priority is 0, which it is for a
  Sim with no running action. `me = "sim"` with `given = { sim = { priority = 50 } }` runs the
  loop; `priority = 0` is a cancelled action.
- **Functionals as trees.** A `functional` compiles to `Can_<Name>` and `Functional_<Name>`;
  run them with `stack_obj = "object"` (the object is the stack object in the chain) and read
  the returned value in `temps.0`.
- **Objects are made off the tile grid.** `given.create` and the subject are not placed, so a
  creation in front of an object (the stove's plate) fails. The fridge and stove specs test
  that failure (the refund); the placed cooking chain (skill caps, nutrition, a cancelled cook)
  is `open_pack_behavioral.rs`'s test.
- **Motives stop at 100**, and skills at 1000 because the objects cap them
  (`helpers::gain_skill`), not the VM.
- **Early stops with `turns`.** A loop that stops once a motive is full ends sooner than one
  that runs every cycle, with the same motive at the end; `expect.turns` tells them apart.
