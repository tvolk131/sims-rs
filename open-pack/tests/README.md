# `open-pack` behavioral specs

Every object in `../pack.toml` has a spec here, `<object>.test.toml`, run by
`sims-antics test open-pack/tests` on a fresh `sims-vm` lot per case. The format is
documented in `docs/decisions/20261124-antics-test-runner.md` (placed rooms, `expect.room` and
`expect.objects` in `docs/decisions/20261126-antics-v2-4-runner-grid-and-if-let-lint.md`);
`cargo test -p sims-antics` runs every spec (`antics_test_runner`) and fails if an object has
none (`open_pack_behavioral::every_open_pack_object_has_a_behavioral_spec`).

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
  room score adds up. Check the word with `data = { 7 = ... }`, and the room itself in a placed
  room (below).
- **Cancel-safe loops through `action`, as the Sim, with a priority.** `cancel_on_abort`
  (and the `std::motives` loops) stop when the Sim's action priority is 0, which it is for a
  Sim with no running action. `me = "sim"` with `given = { sim = { priority = 50 } }` runs the
  loop; `priority = 0` is a cancelled action.
- **Functionals as trees.** A `functional` compiles to `Can_<Name>` and `Functional_<Name>`;
  run them with `stack_obj = "object"` (the object is the stack object in the chain) and read
  the returned value in `temps.0`.
- **Objects are made off the tile grid unless the case asks for the room.** By default the
  subject and `given.create` are not placed, so a creation in front of an object (the stove's
  plate) fails; the fridge and stove specs test that failure (the refund).
- **`room = true` for a placed room.** `[lot] room = true` (every case) or `room = true` on a
  case stands the subject and each `given.create` object on its own tile of an 8-by-8 counted
  room (tiles 4 to 11): `place = [x, y]` under `[subject]`, or
  `create = [{ object = "open-stove", place = [6, 6] }]`, picks the tile; the others take the
  next free tile from `[4, 4]` row by row. Objects face north, so the stove at `[6, 6]` puts its
  plate on `[6, 5]` (`open-fridge`'s placed chain). The room is counted before the run and,
  as the game does before each frame, again after each turn for the rooms marked. A room no
  code marks keeps its old count: a script that changes `room_impact` must mark its room
  (`self.refresh(RefreshMode::RecountRoom)`), and placing an object that is not a light source
  marks nothing. `expect.room = { score, light, lamp_light, light_sources, impact, objects }`
  checks that room's count (`open-floor-lamp`, `open-dining-table`); the tiles around it are
  an outdoor room of their own. The floats are exact, so copy them from a failing run's
  message.
- **`expect.objects` for what is on the lot.** `objects = { open-meal-plate = 1 }` counts a
  loaded object's instances after the run (`0` for one that removed itself); a table,
  `{ count = 1, attrs = { nutrition = 40 }, data = { 29 = 1 } }`, also checks every one of them
  (state names are the object's own). It needs no room, and counts one-tile objects only (a
  multi-tile object is a setup error).
- **Motives stop at 100**, and skills at 1000 because the objects cap them
  (`helpers::gain_skill`), not the VM.
- **Early stops with `turns`.** A loop that stops once a motive is full ends sooner than one
  that runs every cycle, with the same motive at the end; `expect.turns` tells them apart.
