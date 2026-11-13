# Roadmap

The current goal and how work toward it runs. `docs/vision.md` says what we
are building. This file says what we are building *next*, and when it counts
as done.

## Current goal: a headless lot boot that matches the original

Point `sims` at an install. It finds the files, loads one base-game
neighborhood house, runs the objects' setup scripts, and then runs N ticks
with no window. Every run gives the same state hash, and the state matches
the original game, run in the emulator, for the first K ticks.

Done when:

- `sims boot <install> <house> --ticks N` prints a state hash that is the same
  on every run and on both CI platforms (x86_64 Linux, aarch64 macOS).
- For that house, a captured run of the original agrees field for field with
  ours at every compared tick up to K. K starts small and grows. The goal is
  met at K = 1000 ticks, with no unexplained differences.

### Steps

Each step is a few PRs, and each PR leaves `main` green.

1. **Install detection and content lookup.** Recognize a Complete
   Collection install (from its ISO) by its files. Bind every game file and
   resource by name, and fail loudly on anything missing. The layout is
   checked against the real installed tree in the emulator image.
2. **Lot loading.** Read a house file into simulation state: objects, their
   attributes, the scheduler, both RNG streams, the budget and the lot
   arrays. Snapshots save to and load from REDB and survive a round trip
   bit for bit.
3. **Script interpreter core.** BHAV execution with stack frames, the
   operand and variable model, and the primitives that this lot's setup and
   first ticks actually reach. Unimplemented primitives fail loudly and
   name themselves. Arithmetic goes through `sims-core`.
4. **Comparison harness.** Run the original under QEMU and Wine, load the
   same house, advance to tick N, and dump the state we compare. Captures
   are private and are not committed.
5. **Match.** Diff our state against the capture, fix the causes one by
   one, and raise K.

### Progress

- Step 1 is done: `sims install` detects a Complete Collection install and
  builds the object catalog.
- Step 2 is done: `sims boot <install> <house>` loads the Goth house into
  `World`, prints its state hash and can save a REDB snapshot. `--ticks`
  waits for step 3.
- Step 3 is in progress. The interpreter runs saved frames and the
  operations the Goth house's init scripts reach. The load sequence below
  tracks the load steps that must run before the first tick.

### Load sequence

What the original does between opening a house file and running the first
tick, in order, and how much of each step `sims boot` does. The goal
above needs every step: a step skipped or run out of order
changes the state the first tick sees.

The order and the addresses come from the old prototype's research on the
1.91 executable. Each step was checked there on its own, against the
original's code run in isolation. The sequence as a whole has not been
compared with a running game, so it is **unverified** until the comparison
harness exists.

| # | Step | Original | Status |
|---|------|----------|--------|
| 1 | Read the house file: simulator state, type table, objects, lot arrays | `0x415eff` | Done (`World::from_house`) |
| 2 | Set the globals the running game supplies | `0x46cdc6`, `0x46cfb3`, `0x415eff` | Done (`World::apply_context`) |
| 3 | Mark the object types whose scripts changed since the save | `0x43a516` | Computed by `sims boot`, not stored yet |
| 4 | Reconcile global controllers | `0x442b9f` | Not started |
| 5 | Repair the ground object grid and restore placement links | `0x443cb3` | Not started |
| 6 | Give every object its room | `0x442659` | Not started |
| 7 | Discard pending removals | `0x428679` | Not started |
| 8 | Select the household | `0x428041`, `0x46d8dc` | State change done (`World::select_family`), not called yet |
| 9 | Visit every object in registration order | `0x43fc94`, `0x431e2e` | Not started |
| 10 | Create the indicator objects and finish selection | `0x445e4a` | Not started |

`sims boot` runs the steps in order and stops at the first one that is not
done, so it never prints the hash of a state the original could not be in.

#### The steps

**2. Globals.** The simulator zeroes all 64 globals, then overlays the 32 or
64 saved words. It then sets the fields that belong to the running game:
global 15 (mode), 25 (kept from before the load), 29 (always 30), 30 (free
will), 20 (installed expansions) and 33 (hardware performance class). The
speed in global 16 is clamped to 0..1000 and copied to global 18 unless
that is negative. Globals 32 (budget mode) and 9 (family) are cleared.

**3. Changed types.** For each saved type the original compares the saved
Init and Main signatures with the linked scripts. A type that differs is
flagged, and step 9 resets its objects. In the Goth house that is 77 of 113
types.

**4. Global controllers.** Object definitions of type 7 or 12 with a
nonzero global word are lot-wide controllers. The original keeps the first
saved instance of each, deletes later duplicates, and creates the missing
ones through the ordinary object factory, which runs their Init and their
first Main visit. It walks the whole catalog, in catalog order, so the new
objects' IDs depend on that order. The Complete Collection has 141 such
definitions.

**5. Grid repair and links.** Ground-floor grid cells that name an object
which is not at that cell are cleared. Containment and multi-tile links are
rebuilt from the saved slots.

**6. Rooms.** Two passes over the registration list. The first writes each
object's room into data word 29, turning level 0 into level 1. The second
runs each object's function 7.

**8. Household.** Global 9 takes the family number, global 10 the house
number, and the family's money becomes the lot's funds, split across
globals 13 and 14.

**9. Object visits.** For each object, in registration order:

1. A person takes its persistent fields from the neighborhood record
   (`0x4258a3`).
2. Its position is restored, which also places the objects it contains
   (`0x433e50`).
3. A non-person runs its Load function (2). A function whose tree is
   missing is skipped.
4. If its type changed: every person drops references to it (`0x444306`),
   then it resets (`0x431fbe`). An object with a Reset function (3) runs
   it and restarts Main. One without is taken off the lot, has its storage
   reset (`0x431be1`), runs Init, and is put back. A person has its own
   reset (`0x45e791`).
5. If its type changed, it runs Init.
6. Its scheduler flags are refreshed from data word 60 and its
   interaction table.
7. A person updates its cached room and reloads its motive curves.

Lot time does not advance, and no object's saved Main runs, during any of
this.

**10. Indicators.** Two indicator objects are created through the factory
and hidden, and the selected person is found.

#### Known gap before load step 4

The catalog order `sims-content` uses differs from the original's in two
ways the old prototype recovered. Both change which IDs new objects get.

- The original registers definitions in scan order (`0x43c609`): base
  objects, user objects, the neighborhood's people, Downloads, then each
  pack, the shared directories, and Makin' Magic last. `Index` uses the
  mount order (`0x43ae81`) and appends people last.
- Within a directory the original walks its file manager's hash table
  (`0x5ba9bd`), not the file system's order. The table's order depends on
  the lowercase file names and, for names that share a bucket, on the
  order they were inserted.

## Working rules

- Work in branches, open PRs, and merge when CI is green on both platforms.
  Squash merges only, no force-pushes, and run
  `scripts/check_repository.py --history HEAD` before every push.
- Nothing from the original game is committed. Fixtures that need game data
  stay private and are skipped when the data is absent.
- Choices about format details, unverified guesses, and the order of work
  are made in the PR and recorded in `docs/decisions.md`. Guesses are
  marked unverified in the code until the harness confirms them.
- Stop and ask the maintainer only when:
  - something only they can provide is needed (a login, or files not on
    this machine);
  - a choice would change `docs/vision.md` or the product scope;
  - something destructive outside the repository is needed;
  - the original cannot be observed in the emulator closely enough for the
    harness, which means the strategy has to change.
