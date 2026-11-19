# Change Suit/Accessory, and K = 408

*Decided 2026-11-14.*

Tick 368 of the Goth house stopped on Change Suit/Accessory (opcode 6,
`0x45d962`). It is ported in `primitives/change_suit.rs`, with the
animator's "dress" and "undress" (virtual functions 0xd8 and 0xdc,
`0x58d88f` and `0x58d919`) as functions on the person's accessory list,
which the world already keeps from the saved file. Our state then
equals the fourth capture through tick 408 in every field the verifier
compares, with two known exceptions: person 16's head target height is
one ULP low on some ticks, and its head turn a few ULP with it (as
before this step), and the capture holds
the accessory count, not the entry's name or binding. K goes from 367
to 408.

What was decided:

- **A suit no decoded avatar file names fails loudly; any other is
  taken to bind.** The original adds the entry, then binds its suit to
  the skeleton (`0x583559`), storing the name and binding only when that
  works; an entry that fails stays with an empty name. Read from the
  code, the bind fails only when the suit lookup (`0x5a044d`) does: the
  name is not in the avatar registry, or the registry's lazy load of it
  (`0x5a38da`) fails. The attach after it (`0x599427`, through
  `0x5954d8`) has no failure path. The content loader now reads every
  avatar file's skeletons and suits record by record, as the original's
  catalog reader does (so a file holding animations still gives its
  suits), and keeps the suit names (`People::suits`, lowered) and the
  files that stopped decoding (`People::suit_errors`). Putting on a
  name outside that set is `Trap::Unsupported` before anything changes,
  naming any such files.
  Superseded in part by [20261118-change-suit-unnamed-accessory.md](20261118-change-suit-unnamed-accessory.md).
  **Unverified**: that the registry holds exactly
  these names (the install has no NDX index, so where it gets them is
  not read) and that its lazy load succeeds whenever ours decodes. The
  capture shows the entry count (one, from update 368), not the entry.
  Sol asked that every bind fail loudly until the registry is ported;
  rejected, since the code leaves only that registry question, and
  refusing would stop the one case the capture shows.
- **Only the words a branch reads must be known.** Suits 0 to 7 read no
  person data; others read the suit and age, and the variant only on
  the suit already worn. Writing the suit and variant makes them known.
  The age compare is signed (`0x45d9b4`), so a negative age is a
  child's.
- **Clip events "dress" and "undress" still fail loudly.** They call the
  same functions, but how the event's value becomes a name and binding
  (`0x5912fd`) is not read yet.
- **The variant draws use the active stream** (`0x46f457`, as Find Best
  Interaction's draws), so `Machine::draw` is now shared.

What would reopen it: a lot that puts on a suit no skeleton file
defines (it stops loudly now), or a capture page holding the entry.

The first difference is in tick 409, where person 18's route plays its
waiting tree and looks at another person than the original does. It is
silent, not a stop, and is the next step.
