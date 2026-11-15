# A person chooses an interaction by itself, and K = 232

*Decided 2026-11-14.*

In tick 233 of the Goth house, person 21 chooses what to do by itself.
Find Best Interaction (primitive 3) and Gosub Found Action (primitive 30)
are now ported, and so is the first part of Go To Relative Position
(primitive 27). The tick then stops loudly, because the person's route
goes through a door, and routing between rooms is not ported. A second
capture of the original holds the full state after tick 232, and ours
equals it. K goes from 230 to 232.

What was decided:

- **Find Best Interaction is ported** (0x45dd39, `autonomy.rs`). It
  scores each allowed interaction table entry against the person's
  motives, with the person's score curves, in single precision. It sorts
  the candidates with the C runtime's `qsort` (0x6356ba), because the
  order of equal candidates decides the choice. Then it draws once from
  the lot's random stream, unless the best entry or the person skips the
  draw. Its constants come from the global file's tuning table 2, with
  the original's defaults (`Program::autonomy_constants`). The check
  trees it runs can read and change their entry's advertisements (memory
  scopes 12, 28 and 29). What the house does not reach fails loudly:
  free will off for a pet or a visitor, a lot that is not zoned or is a
  community lot, joining another person's interaction, and a visitor's
  entry that asks the relationship.
- **Three primitives that the check trees use are ported**: Alter Budget
  (25), only its test; Test Sim Interacting With (37); and Add/Change
  Action String (50), only inside a check tree, where it does nothing.
- **Gosub Found Action is ported** (0x45a859, `found_action.rs`). It
  pushes the chosen entry's action tree on the person's thread, and the
  person starts using the object. The action becomes the person's
  running action (0x45e6cd, `Machine::set_action`). A new action with a
  target and no number takes the next number from a counter in the
  running game (0x6e4904). No save holds that counter, so it is kept in
  `World::action_serial` and starts at 0. Snapshot format 15.
- **Go To Relative Position builds its slot** (0x45c634,
  `relative_slot`) and then takes the same route steps as Go To Routing
  Slot. A target in another room fails loudly at 0x45b140: the route
  then goes through a door that 0x45c7c2 and 0x46390f choose, and that
  is not ported. A smaller exact K is better than a larger one that is
  not exact.
- **A second capture was made**, to see tick 233. It holds 240 lot
  updates, the full state after updates 1 to 5, 30 to 35 and 225 to 240,
  and a memory image at the entry of update 233. It also watched the
  house view's draw guard, the candidate list, the choice and the action
  counter. The view drew a frame before update 1 and after updates 1, 2,
  3, 33, 63 and every 30th to 213. Our run uses those frames when it is
  compared with this capture. With a frame after every tick from 33
  instead, our state after tick 232 is the same.

How it was checked:

- Our state after each of ticks 1 to 5, 30 to 35 and 225 to 232 equals
  the second capture's, field by field, floats bit for bit, in the same
  fields as for the first capture. The `updates.jsonl` line is equal
  after every tick from 1 to 232.
- The first capture still matches as before: every tick to 100, every
  tenth to 230, and the `updates.jsonl` line to 232.
- In tick 233 the watchpoints saw six candidates, the choice of menu 6
  (0x45e28b), and the action counter going from 0 to 1 (0x41f15e). Ours
  does the same. Find Best Interaction was also compared step by step
  with the original's code, run on a memory image at the entry of
  update 233.
- At our stop in tick 233, these equal the capture's state after the
  tick: the lot's random stream, the action counter, person 21's running
  and last actions, its frames, its use of object 132, object 132's user
  count, and every object that ran before person 21.
- `the_goth_house_first_tick_matches_the_original` runs to tick 232,
  pins the state, and checks where tick 233 stops and what it holds
  there. The pins taken under format 14 still hold with the new counter
  left out (`format_14_bytes`).
- The new primitives have synthetic tests.

What did not match, and why:

- At our stop, person 21's data differs from the capture's. The capture
  went on past the stop: it numbered the person for the doors (object
  data word 6), pushed two frames of the door's tree, and put the door
  in use. Object data word 9 and person data words 27 and 37 also
  differ there. We do not run those steps.
- The first capture does not choose in tick 233. Its one draw of ticks
  233 and 234 comes in tick 234. Its state after tick 232 was equal to
  ours as far as we compared. The second capture, and the original's
  code run on the first capture's state from update 47, choose in tick
  233. So the original does not always do the same here. We follow the
  second capture.
- Global 12 after tick 1 is 17 in the first capture and 0 in the second.
  Ours is 0.

Found wrong along the way: an entry with no check tree failed loudly in
our Find Best Interaction; it is now allowed, as in the original. The
house did not reach that case. The comment on Alter Budget named the
wrong scopes; the code was right (0x44eb02: a literal, an argument, a
local).

**Unverified:** why the first capture chose a tick later; the end of an
action and of a use (no use ends in the ticks that run); whether the
steps the capture took past our stop wrote object data word 9 and person
data words 27 and 37; Alter Budget's spending, and Add/Change Action
String outside a check tree, which fail loudly.
