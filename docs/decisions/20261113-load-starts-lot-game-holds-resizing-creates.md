# A load starts from the lot the game holds, and resizing it creates the controllers

*Decided 2026-11-13.*

The original keeps one lot for as long as it runs and loads each house into
it. Before it reads a house's objects it reads the house's simulator state,
resizes the lot to the house's size, and that resize creates every lot-wide
controller the lot lacks. It then removes every object and only then reads
the saved ones. So 141 controllers run their Init and first turn on a
nearly empty lot, draw random numbers and write globals, and are thrown
away. The load sequence in `docs/roadmap.md` now starts with these steps.
This replaces the ten-step sequence of "The lot load follows a written
sequence" below, which read the whole house first, and that entry's mode 0.

What was decided:

- **`World` is the lot the game holds, not a house.** `World::empty` is the
  lot before any load, `World::read_simulator` reads a house's simulator
  state into it, and `Interpreter::resize` shrinks it. `World::from_house`
  still reads a whole file at once; it is no longer the load path and will
  be split when the saved objects are read onto the cleared lot.
- **The object factory is ported** (`Interpreter::create`): lowest unused
  ID, head of the registration list, the constructor's storage, the date
  stamp, the reset, Main installed, Init run synchronously above it, one
  first turn, then the scheduler flags and the price.
- **Sleep yields.** An object that is not a person and not locked out
  hands its count to the scheduler, which is the low half of the registry
  word. `Interpreter::turn` runs a thread until it yields or returns.
  Resolving data word 25 wakes an object, as in the original.
- **New state.** `World` gains the C runtime's random stream, the lot's
  size, the objects waiting for removal, the type attributes, the per-type
  instance lists, the owner of interface events and the selected family's
  magic history. `Object` gains its last result and a removal flag. The
  snapshot format is 3.
- **Frames of new objects need a type ID.** A frame names its code by the
  lot's type table, which the original only builds when it saves. A type
  the table lacks takes the lowest unused ID. This is our bookkeeping; no
  original state corresponds to it.
- **The whole catalog is linked.** The resize walks every definition, so
  `sims boot` links them all. `Program` also holds each file's interaction
  tables and slot counts, the career list from `work.iff`, and the
  neighborhood's lot zoning.
- **The context's mode is 1.** `apply_context` no longer clears the family
  in global 9; `World::clear_family` does, after the resize, because the
  controllers must still see the saved family.

How it was checked. The old prototype's harness captured the original as it
loaded the Goth house, at the moment the resize returns. Our state was
compared with that capture outside the repository. All 141 controllers
match: GUID, the 72 data words, temps, attributes, last result, every
frame, the registry word and the position. So do the 64 globals, both game
random streams, the tick, the lot size, the registration order, the two
controllers waiting for removal, the scheduler cursor, the funds, the
instance lists, and the type attribute records' GUIDs, order and lengths.
The test `the_goth_house_resize_creates_the_controllers` pins a digest of
our state and runs when `SIMS_INSTALL` is set. The capture is private.

The mode is 1 on the capture's evidence: at the start of the resize, before
any script has run, global 15 is 1 while the house saved 0.

What did not match, and why. Three things in the capture belong to the game
that was running, not to the house or the install:

- The two indicator objects. Ours are created on an empty lot whose clock
  is zero, and have just gone to sleep. The capture's carry another
  creation date and had been asleep for a while.
- The C runtime's random stream, which no file holds.
- Six type attribute words, one for each help system, which are set in
  the capture and clear in the neighborhood file.

With those copied from the capture's state at the start of the resize, the
two indicators match as well, which makes all 143 objects. The controllers
match either way.

What is **unverified**:

- **The starting lot.** The creation date and time the capture's
  indicators carry is exactly the clock saved in `Houses/House00.iff`, the
  empty lot every neighborhood ships with. So the original most likely
  loads House00 when it starts, and the indicators come from that load. We do not model it: `sims boot`
  starts from `World::empty`. This cannot change what a load leaves behind
  unless a controller reads the indicators' state, and none did here.
- **The six type attribute words.** Something in the running game set
  them before the load. We do not know what. The 141 controllers end in
  the same state with either value, but later ticks may not.
- **The C runtime stream's starting state.** It is 1, the runtime's own
  default. Nothing in this step draws from it.
- **Fields the capture does not hold.** A new object's footprint, the short
  after it, its frame arena size, its relationship flag and its slots are
  set to what the original's saves hold for such an object.
- **Paths the 141 controllers do not take.** Sleep, waking, Remove Object
  Instance, Generic Sims Call 28 and 43, event ownership, Set To Next
  modes 4 and 10 and the type attribute scopes are ported from the old
  prototype. The capture confirms the paths these controllers take through
  them, not the others. The original keeps an object with a headline
  showing out of the scheduler's hands while it sleeps; headlines are not
  modeled, so that test is missing.
- **The turn budget.** Init and the first turn may each run 100,000
  instructions, the old prototype's guard. The original's limit is not
  known.
- **Which zoning file the original reads.** We read the neighborhood's
  `LotZoning.iff`; the old prototype read the install's
  `MasterLotZoning.iff`. Both give the same table for the shipped
  neighborhoods.
- **Global 25 and free will in the context.** The capture agrees with
  them, but the house saved the same values, so it proves nothing.

What is not modeled and fails loudly instead: growing a lot; resizing a lot
that has arrays or objects standing on it; a second instance of a
controller, which needs deletion; removing an object that is on the lot,
holds or sits in something, is a person, or has a callback for leaving the
lot; the factories for people, cursors and multi-tile objects; Generic Sims
Call routines other than 28 and 43, and 43 once a family's magic history is
held; a type that wants its attributes cleared for each lot.

What is not modeled and not compared: the lot's rooms and their tile
counts, the interface's action counter, and the interface's lists of
indicators. The resize only writes the indoor area of a bare lot, which is
0, to global 27.

What would reopen this: a capture of the original at startup, which would
settle the starting lot and the type attribute words; a house whose
controllers take the unverified paths; or a load into a lot that is not
bare.
