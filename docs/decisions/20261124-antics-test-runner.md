# `sims-antics test`: live-VM unit tests for `.antics` objects

- **Date:** 2026-11-24
- **Status:** Accepted (Track 7)
- **Scope:** `crates/sims-antics/src/test_runner.rs`, `src/test_runner/`, `src/bin/sims-antics.rs`
  (`test` subcommand), `open-pack/tests/`, `crates/sims-antics/tests/antics_test_{cli,runner}.rs`,
  `crates/sims-antics/tests/test_runner/`

## Context

An `.antics` object's behaviour has been tested in two ways: static checks (`check`, `lint`,
`verify-equiv`), and hand-written Rust tests (`tests/open_pack_behavioral.rs`) that build a
`sims-vm` world, place the object, and step a Sim through a pie-menu interaction. The second
kind is the only one that tests what an object *does*, but every case costs a page of Rust.
Object authors need a quick way to say "with these inputs, this tree returns this and leaves
this state" next to the object, and run it without a game install.

## Decision

### A spec file per object, run on a fresh `sims-vm` lot per case

`sims-antics test <file.test.toml|dir...>` runs `*.test.toml` specs. It is behind the default
`vm-replay` feature; without it the subcommand prints that it needs `vm-replay` and exits 1.
It prints `PASS`/`FAIL <file>: <case>` per case, a failure line per mismatch naming the field,
the expected value and the actual one (`attrs.power (attr[1]): expected 0, got 1`), and a
summary `N passed, M failed` (plus `K file(s) could not run` for specs that do not load). It
exits 1 on any failure.

Specs are read with the crate's own TOML subset (`manifest::parse_toml`): no dotted keys and
no multi-line strings, and inline tables may span lines. Unknown keys are errors everywhere,
and so is one value given twice under two spellings: `15` and `fun`, `0` and `00`, a state's
name and its index, `person.58` and `age`, `skills.cooking` and its person word. (A case's
`given.sim` still overrides the file's `[sim]`; the rule is per table.) The spec, the subject
and every stub `file` must be regular files, so a FIFO or a device cannot hang the run: the
path is checked, then opened, and the opened handle is checked again, so what is read is what
was checked. The open does not block (`O_NONBLOCK`, as the v2 loader's
`read_regular_file_checked` does) on the targets whose flag value the runner sets: macOS, iOS
and the BSDs, and Linux and Android on x86, x86-64, ARM, AArch64 and RISC-V 64. There a FIFO
swapped in between the check and the open cannot block it; on any other target it can.

```toml
[subject]
antics = "../objects/open-tv.antics"   # or: iff = "compiled.iff"
pack = ".."                             # where `use crate::...` resolves
idle_main = true                        # its Main returns at once: run with an idle loop
                                        # in its place (an ASSUME line per case that does; below)

[lot]
also = ["../objects/open-fridge.antics"]   # loaded too; created by `given.create`

[[stub]]                                  # global (256-4095) or semi-global (8192+) trees
id = 300
returns = true                            # or: source = "tree 301 \"R\" (args: 1) { ... }"
                                          # or: file = "stubs.air"
[sim]                                     # a Sim on the lot for every case
motives = { fun = 0 }                     # also skills, personality, age, priority,
                                          # person, attrs, temps, data (by index)
[[test]]
name = "watching restores fun"
interaction = "Watch TV"                  # or tree = 4100 / "Label", guard = "...", action = "..."
turns = 60
answers = [1]                             # dialog buttons, in order
given = { attrs = { power = 0 }, funds = 100, create = ["open-fridge"] }
expect = { attrs = { power = 1, watch_cycles = 4 }, sounds = ["open_tv_click", "open_tv_jingle"],
           sim = { motives = { fun = 96 } }, dialogs = [], queue = 0, funds = 100 }
```

- **Targets.** `tree` runs one of the subject's trees by ID or label (its own file first, then
  the stubs); an ID with no tree, or a primitive's, is a setup failure in either mode. `guard` and `action` run the check or action tree of the TTAB entry whose TTAs
  name matches. `interaction` makes the Sim pick the entry from the subject's real pie menu
  (`Interpreter::pie_menu`, `pick`), keeping the target the menu gives it (the lead tile of a
  multi-tile object), and runs the Sim's thread until the action finishes. It selects the
  entry by the name the menu shows, as `sims-game`'s menu (`menu::entry_label`) builds it:
  the non-empty name its check gave it (Add/Change Action String, `MenuEntry::name`), or else
  its TTAs label, or `Interaction <n>` for a blank or missing one; a debug entry (TTAB flag
  0x80) gets `*` before that, unless it is a join offer. An entry a check renamed, or a debug
  entry, is not offered under its table name, and the failure says what it shows instead.
  `guard`, `action` and `queue` keep the table names. A name that neither the menu shows nor
  the subject's menu can hold (the table of the subject's definition, or of a tile of its
  multi-tile group, by TTAs name or the name the menu shows for an entry) is a setup error
  listing both. So is such a name when listing the menu faults; the failure also gives the
  fault. Another definition of the same file is not the subject's menu: its names do not count.
  TTAs names are each definition's own: the subject's for `interaction`, `guard` and
  `action`, and the target's for an action `queue` names, since two definitions in one file
  each number their menu from 0. A definition's table is the one `Program::interactions`
  uses: its word 7, or for a tile whose word 7 is -1 its group master's, in its own file or
  else its semi-global file; the names are that file's TTAs of the same ID.
- **Inputs.** Subject attributes (by state name or index), temps, data, lot globals, funds
  (the budget and globals 13/14 together), objects to create, and the Sim's motives, skills,
  personality, person data, attributes, temps and data. A case's `given.sim` is layered over
  the file's `[sim]`. `params`, `me` (`object` or `sim`) and `stack_obj` (`object`, `sim`,
  `none` or an ID) set the run's frame; `budget` is a synchronous call's instruction budget
  (default 100,000). A threaded run's turns are lot updates under the game's own limit, so a
  `budget` there is a setup error.
- **Results.** `true`, `false`, `error` (any fault, or the thread's object leaving the lot;
  `error = "text"` also matches the message), `ok` (no fault), `yield` (started and still running when its turns ran out) and
  `unavailable` (the pie menu did not offer the interaction, the queue did not take it, or
  the Sim dropped it or never started it in its turns). A case that expects no result still
  fails on `error`, `yield` or `unavailable`.
- **State.** Subject attributes, temps and data; lot globals; funds; the Sim's motives
  (compared as exact floats), person data, attributes, temps and data; `stack_obj` (the
  outermost frame's stack object as the tree returned, read from the frame the VM saves when
  the frame is popped); `turns` taken.
- **Effects.** `sounds` (the `Play` events' names, in order), `dialogs` (the kinds made, in
  order: `ok`, `yes_no`, `yes_no_cancel`, `text` or a number; or a count), and `queue` (the
  actions added to the Sim's queue by the run, each with any of `interaction`/`menu`, `target`,
  `priority`; or a count).

### The lot is loaded through the real loader

Per file, the subject and `[lot].also` are compiled in memory
(`v2::compile_v2_object_file`) or read (`.iff`), and with a `Global.iff` holding the global
stubs and a semi-global file holding the semi-global ones they become in-memory files of a
`sims_content::Index` (`add_character_files`). `Catalog::build` registers them and
`sims_vm::load_library` loads them: the same code a game install goes through. Semi-global
stubs take the place of the subject's own semi-global link. The only hand-built parts are the
Sim's type (a person definition whose main tree is an Idle For Input loop, GUID `0x7E570051`)
and the world: the flat 64-by-64 single-room lot of `tests/open_pack_behavioral.rs`, with
the Sim as object 1 and the subject made by `Interpreter::create` (which runs its Init).
Attribute names come from the `// Attributes:` header of the object's v2 expansion; an `.iff`
subject's attributes are named by index only. Every case gets a fresh world.

### Three ways to run, by what the case needs

1. **A synchronous call** (a tree, `me = "object"`, no `turns`): `Interpreter::run_tree`, as
   the game runs a callback; a tree that yields is a fault there. `stack_obj` is not
   observable in this mode.
2. **A thread** (`me = "sim"`, or `turns` given): the tree is the only frame on that object's
   stack, with the subject's code and the frame a call to it gets
   (`Behavior::frame_dimensions`: format 0x8000 always has four arguments, 0x8001 caps both
   counts at four). The scheduler countdown the replaced thread left is cleared (-1, as the
   factory leaves it), so the tree's first Sleep is its own. Each turn is one real lot update
   (below). More than 255 `params` fault in both modes, as `run_tree` refuses them before it
   sizes the frame. A Sim run without `turns` gets one turn, and a yield is reported with a
   hint to set `turns`. Indexed (kind 4) trees run only as calls. The thread returning from
   its outermost frame is error 0x3e9, which stops that update (its handling, the object's
   virtual function 4, is not ported): the run ends there with the returned result. The stack
   object the tree returned with is read from the popped frame the VM keeps in
   `World::frame_storage` (word 2), so a tree that changes it on its last instruction is seen
   as it left it. If the end of an update removes the thread's object (it queued its own
   removal), the case is an error: the game would not run that thread again.
3. **An interaction** (`interaction = ...`): the pie menu is listed (`pie_menu`; the entries'
   checks run), the selected entry is checked again as the game does when the player selects
   it (`Interpreter::highlight`, 0x48df28; `pick` gets the checked action), and `pick` queues
   it. If that check faults, the case fails whatever it expects: the game runs it on a copy of
   the lot, drops the copy, and still picks the entry as listed (`sims-vm`'s `Lot::highlight`,
   `sims-game`'s `check_entry`), and what the original does is not known, so the runner
   follows neither. The Sim's main loop then runs it, one lot update a turn, default 60 turns, and the
   check when the action starts is the main loop's own. Hover highlights (the game's check as
   the pointer passes over an entry) are not modelled; only the selection's check runs. The
   runner watches the Sim's active and last action after every instruction of the Sim's turn,
   on a copy of the world stepped ahead of each update: the case is done once the action has
   started and ended, even when a successor it pushed to the front of the queue then runs and
   ends within the same turn. The copy's view is used only when the real update ticked the
   Sim first, as it does (object 1). An at-once entry (TTAB flag 4, read from the target's
   table as `pick` reads it) runs inside `pick` and is done there with 0 turns. `pick` then
   returns the number of a successor it queued, if any (the successor is copied over the
   running action, 0x45d5a5), so the runner never follows that number: what the action queued
   stays queued and `queue` lists it. A number `pick` returns for an entry that does not run
   at once must be in the queue, or the case fails.

### Between turns: the real lot update

Each turn is `sims-vm`'s own `Interpreter::lot_update_kept` in strict mode (no `Play`). The
scheduler's pass ticks every object in ID order: `tick_object`, or `tick_person` for the Sim,
with its decay, animator, Room from the room's score, person word 27, the motive changes over
time and its usage sample. The clock advances on a full pass, and `end_of_update` shows the
dialogs and drains the queued removals. The runner keeps no tick code of its own. The Sim's
type has definition word 102 bit 2 set, as `sims-vm`'s own fixtures' people do, so its
animator does not start a posture's base clip (the type has no animations or profile).

Whatever the case expects, it fails, naming the cause, when the update does something the
runner's lot cannot model:

- **Decay.** The person tick decays the Sim's motives every 60 ticks, from the global
  file's tuning and mood curves, which the test lot does not have (the rates are the
  original's defaults, `Rates::from_lookup`, and there are no curves). A turn in which the
  person tick reaches that point (its last-decay tick, `World::person_ticks`, changes) stops
  the run, so a run with a Sim gets at most 60 turns. That holds with decay switched off too
  (person word 29, or data word 34, not 0): the tick reads both after the Sim's own turn in
  the same tick, so a script can switch decay on just before it, and the words as they stood
  before the update cannot say whether the motives decayed.
- **Other objects' faults.** A fault of another object stops the update; live play would
  record it and reset the object, which strict mode does not do. An object whose Main
  returned on its first turn, when it was created, has no thread for its tick: the case stops
  on it and says so (for the subject, with a hint to set `idle_main`). `.antics` v2 gives an
  object with no `main` a Main of `return true;`, so such an object needs a `main` that loops,
  or the spec's explicit `idle_main` (below).
- **Dialogs.** These errors are strict mode's: `end_of_update` (0x436da0) raises
  `TickError::Unanswered` for an ordinary dialog with no answer or one it does not offer,
  `TickError::UnansweredSpecialPanel` for a special panel, and `TickError::Unsupported` for a
  kind it does not port or a dialog that does not stop the game. Live play may instead record
  an unsupported dialog as a recoverable script error and go on; the runner does not, and
  reports the dialog's kind and buttons. A dialog is answered as the player would
  (`Answers::choices`): one with a single button that is not a text prompt answers itself;
  otherwise the next entry of `answers` is used, which must be one the kind offers (ok: 1;
  yes/no: 1-2; yes/no/cancel: 1-3; text: 1). Dialogs made where no update follows (listing
  the menu, the pick, a synchronous call, or a run that stopped inside an update) are shown
  by `end_of_update` as after a partial pass, which leaves the removals queued. `dialogs`
  gets each one's kind. A dialog that only informs is made, shown and closed within one
  update and leaves no record, so when a case expects `dialogs` the runner first runs the
  update's scheduler pass on a copy of the world. The case fails if the real update showed a
  different number.
- **Motive changes with no update after them.** A run can end with motive changes over time
  pending and no lot update after the code that set them (a synchronous call, or an
  interaction that ended before its first turn). It cannot assert the Sim's motives, because
  the person tick that applies them did not run.

### `idle_main`: an explicit, reported stand-in for a Main that returns

A Main that returns at once is a real bug in the object: the game's lot update faults on the
object's first tick (error 0x3e9, a thread returning from its outermost frame), and live play
resets the object and then parks it. Every `open-pack` object has one today (v2's default
Main, or a written `return true;`), so every interaction case failed on it once the runner
moved onto the real lot update. Hiding that would make the runner pass objects the game
cannot run; failing every case would leave the objects' interactions untested until the
Mains are fixed. The runner takes neither: a spec opts in, visibly, per object.

- `[subject] idle_main = true` (default false), and `idle_main = true` in a `[lot].also`
  table entry (`also = [{ antics = "...", pack = "..", idle_main = true }]`; the bare-path
  form has no key). The key covers that object's own definition (the subject's, or the `also`
  object's GUID: the compiled object's, or an `.iff`'s first definition) and, for a tile of a
  multi-tile object, every definition of its group (OBJD word 10), which one creation makes
  together. Other definitions of the same file are not covered. Whenever a case creates
  objects of a covered definition (the subject, `given.create`, or anything their Init or
  Main makes), each whose Main returned on its first turn in the factory gets the idle loop
  (`while 0 == 0 { sleep(1) }`, Sleep of one tick) as its only frame, as the factory
  installs a Main (0x47480c: four zero arguments, no stack object), with its countdown
  cleared (-1). The lot update then ticks it like any sleeping object. Other objects are
  untouched: an object without the key still stops the case, naming it.
- **Nothing in the lot can call the loop.** It is the only tree (4096, labelled
  `TestRunnerIdleMain`) of a synthetic library of its own, whose one definition (GUID
  `0x7E571D1E`) no object is made from (`given.create` refuses it, and the test Sim's GUID, as
  reserved); the replaced frames name that definition's type
  (32767, registered when a Main is replaced; the factory gives a new type the lowest unused
  ID) as their code's owner. A call resolves a private tree in its caller's own file, so a
  script calling the loop's ID, a spec's `tree = 4096` or `tree = "TestRunnerIdleMain"`, and
  Run Tree By Name all see exactly what they would without the key.
- **It is loud.** A case reports each covered definition whose Main it replaced
  (`CaseReport::assumptions`: `subject Main (OBJD #128, GUID 0x7f000123) replaced by an idle
  loop (idle_main)`, `open-fridge Main (OBJD #128, GUID ...) replaced by ...`), and the CLI
  prints each as `ASSUME <file>: <line>` before that case's `PASS`/`FAIL` line. A PASS holds
  only under them. A key that a case never acts on (an `also` object it does not make) is
  not reported.
- **It cannot mask a different Main.** If a creation's objects of covered definitions all
  still have a thread after the factory (their Main loops or sleeps), the case is a setup
  error: `idle_main is only for a Main that returns: <owner> (OBJD #<n>, GUID <guid>)'s Main
  is still running after its first turn; drop idle_main`.
- **A `tree = "Main"` case still runs the real Main** (as a call, or as a thread whose only
  frame is Main), so `open-mailbox`'s "the main loop returns at once" keeps testing it.
- All four `open-pack` specs set it on the subject, and `open-wall-phone` on the
  `open-fridge` it restocks.

**Follow-up (Track 8, the v2 compiler):** the default Main v2 gives an object with no `main`
should loop (`loop { sleep(1); }`), or the compiler should require a `main`. Fixing the
objects instead is the alternative: `open-pack-looping-mains.patch` (kept with the runner's
review notes) gives each `open-pack` object a looping `main`. Once the objects loop, the specs
drop `idle_main`, and then `idle_main = true` on them is a setup error, so a stale key cannot
linger; "the main loop returns at once" then needs a new expectation.

### Tests run in CI without a game install

Everything committed is synthetic or the repository's own `open-pack` sources. Specs for
`open-tv`, `open-wall-phone` (dialog answers, funds, `[lot].also`), `open-shower` (wear,
breakage, skill-scaled repair) and `open-mailbox` (budget) live in `open-pack/tests/`. A
synthetic fixture with its spec (global stubs and a pushed interaction) lives in
`crates/sims-antics/tests/test_runner/`: it needs `@id` for its `extern mod`, which
`open-pack`'s pure-v2 rule forbids, and `**/tests/fixtures/` is ignored by git. All of them
run from `cargo test`, through both the library and the built binary.

## Consequences and follow-ups

- **A hand-built lot.** The lot is one flat room, the Sim is a bare person with no
  animations, and the global file holds only the stubs, so decay (above) and animation are
  not the game's.
- **Strict mode.** The runner never uses `Play`, so it never models the game's recovery from
  a script error (reset, then park). A case that needs it fails, naming the error.
- **Recursive creation.** `sims-vm`'s factory runs a new object's Main for its first turn
  inside the creation, with no depth limit, so a Main (or Init) that makes another object of
  its own kind nests creations until the lot runs out of object IDs (32,767). On a default
  stack that overflows and aborts the process. The runner runs each case on a thread with
  2 GiB of stack (only the pages used are committed; about 0.5 GiB in a release build and
  1 GiB in a debug one, measured), so such a case fails on the factory's `no unused object
  ID` instead (after about 12 s in release, 140 s in debug), and a failure line longer than
  4,000 characters keeps its start and end. If the thread cannot be made (strict overcommit,
  a tight address-space limit), the case runs on the caller's thread and reports an
  `ASSUME` line saying so, since such a case may then overflow that stack. **Follow-up (`sims-vm`):** a creation-depth guard in the
  factory (the original's own behaviour for nested creation is not known), which would make
  such a case fail at once and not depend on the stack size.
- **`open-pack` objects need a Main that loops.** Every `open-pack` object's Main returns at
  once (`return true;`, written or given by v2 for an object with no `main`), which the real
  update faults on at the object's first tick; in live play the object is reset and then
  parked. Until each object gets a `main` that loops (or v2's default Main loops; the Track 8
  follow-up above), the `open-pack` specs run with `idle_main`, and every case of them prints
  that assumption. With looping Mains every `open-pack` case passes without the key except
  `open-mailbox`'s "the main loop returns at once", which tests that returning Main.
- **Guards run as plain calls.** The game runs a check tree on the Sim's stack with the
  checking flag set (`call_as`); that entry point is crate-private in `sims-vm`, so `guard`
  runs the check tree as a thread of `me` (the Sim) with the object as stack object, which
  matches every check tree that does not read the flag. `interaction` cases go through the
  real pie menu and so through the real check.
- **Only the subject's state is asserted.** Attributes of other created objects (the fridge
  the phone restocks) cannot be expected yet; an `expect.objects` table is the natural next
  step.
- **State names are read from the expansion header.** A public v2 API for an object's state
  layout would replace the text scan.
- **No randomness control.** A tree that draws random numbers runs from the empty world's
  initial random state; a `given.seed` would make that explicit.
- `push_interaction(priority: ...)` in `.antics` takes the raw priority code (3 is 50), as
  the primitive does; the fixture's spec documents it.
