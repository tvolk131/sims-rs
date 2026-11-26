# `.antics` v2.4 DX: placed rooms in `sims-antics test`, `expect.objects` / `expect.room`, `prefer_if_let`

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/test_runner.rs`, `src/test_runner/{harness,tests}.rs`,
  `src/v2/lint.rs`; lint tests appended to `tests/integration/v2_2_step2_lint.rs`;
  `open-pack/tests/**` (four specs, the README). No change to lowering, the parser, the
  open-pack objects or `v2/decompile.rs`.
- **Track:** Track 7, `.antics` v2.4
- **Builds on:** [`20261124-antics-test-runner.md`](20261124-antics-test-runner.md) (the
  `.test.toml` format), [`20261126-antics-v2-3-step2-lint.md`](20261126-antics-v2-3-step2-lint.md)
  (`if let` in the lint)

## Decision 1: a placed room is opt-in, per file or per case

Every existing spec builds its objects off the tile grid, and two of them (the fridge's and
the stove's refund) test exactly the failure that gives: a creation in front of an unplaced
object fails. So placement never becomes the default. All 148 specs that predate this run
unchanged: every new placed case opts in on its own, and the dining-table spec's new
`[lot] also` (the trash can) only loads an object for `given.create` to make.

- `[lot] room = true` places every case of the file, and `room = true` on a `[[test]]` places
  that case alone (`room = false` there takes a file-wide room back).
- **The room.** `empty_world` makes room 1 an 8-by-8 counted room on tiles 4 to 11
  (`ROOM_TILES`) at level 1, inside the 16-tile lot. Its first tile has a covered west wall,
  as the behavioural suite's placed rooms have, so its score is finite. The rest of the
  ground level is an outdoor room 2, so the room map and room 1's tile list agree: an object
  a script moves or makes off the room's tiles is in room 2, not in an uncounted corner of
  room 1. Without `room = true` the lot is as before (every tile in room 1, no tiles listed).
- **Placement.**
  - The subject and each `given.create` object stand on a tile of their own, facing north.
  - `[subject] place = [x, y]` and `create = [{ object = "...", place = [x, y] }]` pick a
    tile; a bare `create` string still works.
  - Unpicked objects take the next free tile from `[4, 4]`, row by row, after every picked
    tile is reserved.
  - A tile outside the room, two objects on one tile, a full room, or a multi-tile object is
    a setup error naming the case, never a quiet fallback. So are a tile whose object-grid
    head a script already took (an Init that made an object there), and an object whose Init
    already placed it on another tile (moving it would leave its old grid head behind, and
    the room would count it twice).
- **Room counts.**
  - The harness marks both rooms (room 1 and the outdoor room 2) and runs the VM's room
    update (`update_pending_rooms`) once before the run.
  - As the game does before each frame, it runs the update again after each lot update and
    after the run.
  - A room is counted again only when something marks it, as in the game: a script's
    `self.refresh(RefreshMode::RecountRoom)`, or the VM's placement and move code, which
    marks the rooms an object with a room impact (data word 7) or the light-source flag
    enters and leaves. So a newly created plate with neither is not counted until something
    marks its room, and a script that changes `room_impact` in place must mark the room
    itself. The specs pin this rather than hide it.
  - The Sim stands on tile `[2, 2]`, in the outdoor room 2, and its room word (data word 29)
    says so unless the case gives one. Without `room = true` it is in room 1, as before.

## Decision 2: `expect.room` and `expect.objects`

- **`expect.room`** is `{ score, light, lamp_light, light_sources, impact, objects }`, all
  optional, read from the fixture's room 1 as the last room update counted it (not from
  wherever the subject ends up: a subject that removed itself still has a room to check). `score` and `light` are compared
  exactly as `f32`: the room math is deterministic, and a tolerance would hide a changed
  formula. A case without a room that asks for `expect.room` is a parse error.
- **`expect.objects`** maps a loaded object's slug to a count (`open-meal-plate = 0`) or to
  `{ count, attrs, data, temps }`.
  - The table checks every instance on the lot after the run, by the object's own
    attribute names.
  - With no `count`, it asks for at least one instance.
  - It needs no room.
  - Each failure names the instance (`objects.open-meal-plate (object 12).attrs.nutrition`).
  - A slug that is not loaded, or an attribute name the object lacks, is a setup error that
    lists what exists.
  - A multi-tile object is a setup error: its tiles carry GUIDs of their own, so counting
    the slug's GUID would count one tile of each instance, or none. Counting groups waits for
    a spec that needs it.
- **New cases.**
  - `open-floor-lamp`: a new lamp lights its room; turning it off darkens the room, and
    turning it on lights it again (each switch recounts the room).
  - `open-dining-table`: setting a placed table raises its room's score, tidying it drops the
    score, and the table counts with the trash can beside it.
  - `open-fridge`: the placed chain cooks a plate in front of the stove.
  - `open-meal-plate`: an eaten plate leaves the lot.
  - With these, the runner holds 156 specs, all passing.

## Decision 3: `prefer_if_let`

The rule finds code that names an `Option` call's value by its temp.

- **What it matches.** `if f(..) { .. temp[N] .. }`, or `if !f(..) { <leaves>; } .. temp[N] ..`,
  where `f` is an `#[ret(temp = N)] -> Option<T>` tree. That is a tree or `extern` function of
  the file set (definitions that disagree count as unknown), or a `std::globals` tree when the
  file imports `std::globals`.
- **The rewrite.**
  - The positive form becomes `if let Some(v) = f(..) {`.
  - The negative form becomes `let Some(v) = f(..) else { .. };`. It needs no `else` of its
    own, an `else` body whose last statement always leaves, and no `else if` above it.
  - The reads become `v`. The help lists each renamed read as `line:column`: the
    suggestion is the binding alone, and a later `temp[N]` may hold another value.
  - Comments inside the leaving block are kept: the fixes go around the block, not through
    it. The block's end is found by the lexer's tokens, so braces in comments (nested ones
    too), strings and `raw` blocks do not count.
- **The name.** `v` is the callee's last word (`find_companion` gives `companion`,
  `take_elapsed_periods` gives `periods`), or `value` for a one-word callee or a keyword. It
  takes `_2` .. `_9` when the list already names or declares that word (an inner `let` would
  hide the binding), a name in scope does, or a function of the set does.
- **Which reads are renamed.** Only reads in the statements before the first one that may
  change a temp. Such a statement holds a call, a method call, `?`, `&mut`, a block
  expression, a `raw` block, a loop or block statement, a `let Some` / `if let`, or a write
  whose target names `temp`. Later reads keep `temp[N]`. With no renamed read there is no
  finding.
- **Byte check.** Every rewrite goes through the lint's trial build. The binding lowers to a
  frame word (`let __v_v_0 = temp[N];`), and the backend folds it away only when the value
  goes straight into one store (`forward_copies`: `stack_obj = companion;`,
  `slot_1 = companion;`). Every other shape builds other bytes, the trial refuses it, and
  the rule stays silent.
- **What this means in practice.** The rule fires where the rewrite is free. Making every
  read free is lowering's job: binding an `Option` value to its temp while nothing
  clobbers it. That is a Track 8 follow-up, not something to work around here.

### Corpus

Run on the Track 9 v2.3 lifts (1,191 files each), against `lint --json` baselines taken with
the base commit's lint (`d097b1b9f`), keyed on (file, line, column, rule, message):

| Corpus | Proposed | Kept (byte-identical) | Other findings changed |
|---|---|---|---|
| `lift-v2-3-beh` | 148 | 16 (12 `let .. else`, 4 `if let`; all `find_companion`) | 0 |
| `lift-v2-3-strict` | 26 | 0 | 0 |

All 16 were applied together per file (6 files). Each rewritten pack builds a FAR
byte-identical to the original, passes `fmt --check`, and has no `prefer_if_let` left: no
false positives and no broken rewrites. The other 132 proposals in `beh`, and all 26 in
`strict`, build other bytes and are dropped. Typically the value goes into a compound store
(`self.attr_2 += temp[0]`) or is read after other statements, so the binding keeps its frame
word.

## Consequences

- Specs can now pin room-score and light behaviour and object lifetimes, which before needed
  hand-written `PackVmFixture` tests in `open_pack_behavioral.rs`.
- The runner's room is one fixed room. Multi-room lots, levels other than 1, and multi-tile
  objects stay out of scope (setup errors) until a spec needs them.
- `prefer_if_let` recall rises without lint changes once lowering binds `Option` values
  without a frame word.
