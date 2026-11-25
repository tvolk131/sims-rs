# `.antics` v2: strongly typed `std` foundation

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/std/`, `v2/{ast,parser,loader}.rs`, `v2/lower/*`,
  `crates/sims-antics/tests/integration/v2_typed_std.rs`
- **Track:** Track 8 (v2 front end and lowering), `.antics` v2.1 Wave 1 Stream 1
- **Follows:** `20261125-antics-v2-rust-like-syntax.md`

## Context

Engine words were plain `i16`. Code read `self.data[13] flag_set 6` or `global[20] flag_test 6`, and
nothing named what a bit or value meant. Only `Height` and `Placement` were typed, and both were
built in by hand. `std::globals` declared 17 trees.

The goal is typed engine values with three limits:

- **No cost.** A typed spelling builds the same nodes as the number it replaces.
- **Back-compatible.** Every existing source keeps compiling: the open pack, the pilots and the
  lift corpus.
- **Clean-room.** The names are our own descriptions. Only word, bit and value positions are engine
  facts, and every one is checked against `crates/sims-vm`.

## Decision 1: `std::flags` and `std::engine` modules

Two new built-in modules sit next to `std::globals`:

- **`std::flags`** (bitflags, 1-based bits):
  - `Height` (data 4) and `Placement` (data 42);
  - `ObjectFlags` (data 8), `WallAdjacency` (data 5), `WallPlacement` (data 13);
  - `ObjectFlags2` (data 40), `BuyFlags` (data 43);
  - `Expansions` (global 20).
- **`std::engine`** (enums):
  - `Direction` (data 1, eighths clockwise from north);
  - `BudgetCategory` (6 and 7 only);
  - `RefreshTarget`, `RefreshMode`;
  - `ObjectType` (definition word 9);
  - `PersonKind` (person 65).

Only bits and values the VM reads or writes are named. An unconfirmed bit is left out and stays
reachable through the raw word. Two exceptions: `Height` keeps its eight historical names, and
`Placement` gains bits 11 and 13.

`Height` and `Placement` are **prelude** types: every module still sees them with no `use`.
`use std::flags::{Height, ..}` and `use std::flags::*` name the same types, because the loader
skips re-importing a reserved prelude name. Only a built-in `std` module may declare a reserved
name (`parse_v2_std_file`).

## Decision 2: zero-cost conversions

- **`expr as i16`** is a postfix unary (`V2UnaryOp::AsI16`).
  - It accepts `i16`, enums, bitflags and constant `bool`s. A runtime `bool` is refused, because a
    bool word may hold any nonzero value; write `if b { 1 } else { 0 }`.
  - A chain `x as i16 as i16` is one node.
  - It rejects object handles, so `stack_obj as i16` is an error.
  - Const evaluation folds it.
  - Only `i16` is accepted as a target: `as u8` is a parse error.
- **`T::from_raw(expr)`** works for any bitflags type and any `std::engine` enum `T`.
  - It takes exactly one positional `i16` (or a `T`).
  - It lowers to its argument.
  - A user enum is refused: it is closed, and a word with no variant would match a listed arm.

Neither adds a node, and a comparison reads either one in place, as it reads the value it converts.
A constant primitive operand takes a cast of a constant: `-32768 as i16`, `Direction::from_raw(1) as i16`.
A declared callable at the same path wins over `T::from_raw`, for example an `extern mod Height`
with a `from_raw` fn.

## Decision 3: engine words are lenient; engine enums are open

- **A `std::flags` type, or the untyped `self.flags` word, is an engine word.** It takes raw `i16`
  operands exactly as before:
  - `w = 5`, `n = w`, `w & 4`, `w > 0`, `w flag_set 3`, `w flag_test 2`.
  - Two words of different typed kinds compare with neither `==` nor `<` until they are cast.

  This is what keeps about 2,000 numeric `self.wall_placement flag_set N` uses in the lift corpus
  compiling. **User bitflags stay strict.**
- **An `i16` cell takes a typed single-flag constant** of any bitflags type in
  `flag_test`/`flag_set`/`flag_clear`, for example `world.expansions flag_test Expansions::Pets`.
  A multi-bit mask is an error, and so is a constant that mixes two bitflags types. Past the
  constant path, a flag operation takes only an `i16` bit number. A runtime bitflags value is a mask,
  not a bit, so it is refused.
- **Engine enums are strict in assignments.** `self.facing = 3` is an error; use
  `Direction::from_raw(3)`.
- **Engine enums are open.** A word may hold a value with no variant, so a `match` on a
  `std::engine` enum needs a `_` arm, even when every variant is listed. User enums stay closed.
  The verifier's `--v2-enum-domains` therefore assumes no domain for a `std::engine` parameter.
  This is a 2-line change in `verify/domains.rs`.
- **Primitive operands take engine constants as their numbers.** This covers any constant
  primitive operand: `refresh(target: RefreshTarget::StackObject, mode: RefreshMode::RecountRoom)`,
  and the `category:` of `can_afford`, `debit_funds` and `credit_funds`.

## Decision 4: one typed engine-word table

`lower/tree.rs` now holds the named words as `EngineWord { index, ty, writable }`. The same table
serves `self.*`, `sim.*`/`me.*`, typed handles, untyped handles (`stack_obj.*`) and the
alias check.

| field | word | type |
| --- | --- | --- |
| `facing` | data 1 | `Direction` |
| `container` | data 2 | `ObjectId`, **read-only** |
| `allowed_heights` | data 4 | `Height` |
| `wall_adjacency` | data 5 | `WallAdjacency` |
| `room_impact` | data 7 | `i16` |
| `flags` | data 8 | any bitflags (`ObjectFlags`) |
| `wall_placement` | data 13 | `i16`, as before; takes `WallPlacement` constants |
| `broken` | data 15 | `i16` |
| `weight`, `support_strength` | data 27, 28 | `i16` |
| `flags2` | data 40 | `ObjectFlags2` |
| `value` | data 41 | `i16` (current, depreciated value) |
| `placement` | data 42 | `Placement` |
| `buy_flags` | data 43 | `BuyFlags` |
| `category` | data 59 | `i16` |
| `sim.gender_kind` | person 65 | `PersonKind` |
| `sim.date_partner` | person 77 | `i16` (a store queues the date indicator) |
| `world.expansions` | global 20 | `i16` (the `Expansions` mask) |

Notes on the table:

- **`wall_placement` stays `i16`.** It was already a named `i16` word, and numeric code uses it
  as one, for example as the destination of `random_into`. As an `i16` word it still takes typed
  single-flag constants. The words that are new in this change are typed.
- **A state of the same name wins** over an engine word, for `self` and for typed handles. Existing
  objects with `state value` keep their meaning.
- **`container` is read-only.** The engine traps a script store of a different value into data
  words 2, 3 and 11. Writing `container` is a compile error.
- **Names differ from the original request where the VM disagreed:**
  - Data 42 holds placement flags, not "movement flags". The movement and intersection bits are in
    data 8.
  - Data 43 holds Buy-mode flags, so the field is `buy_flags`, not `flags3`.
  - Data 40 is `flags2`.
  - `world.selected_sim` (global 3) already existed.
  - Global 20 is the installed-expansions mask, now `world.expansions`.

## Decision 5: named and numbered id/room words build the same nodes

Reads of `my.data[11]`, `stack.data[11]`, `my.data[29]` and `stack.data[29]` now lower to
`my.id`, `stack.id`, `my.room` and `stack.room`:

- Each pair encodes the same operand.
- The optimiser knows `stack_obj := my.id` as a value, so `stack_obj = sim.data[11]` is now
  optimised exactly like `stack_obj = sim.id`.
- `sim.id`, `self.id`, `sim.room` and `self.room` keep their `.air` spellings.

Tests compare the `.iff` bytes of named and numbered sources.

## Decision 6: `std::globals` grows to 62 trees

The 45 new entries come from a survey of each tree's live code and its callers in the Complete
Collection install. Each entry records:

- the parameters the tree really reads;
- whether the false exit is a real answer;
- any temps a caller reads afterwards. Two trees answer in `temp[0]`: 282 (whole periods) and 333
  (the companion).

The rules for entries:

- Each entry stays on one line, because the lift reads the file line by line.
- All parameters are `i16`. A 0/1 switch is documented in the entry's comment. This keeps lifted
  literal calls compiling.
- Names are our own. None copies an original label.

Changes to the existing entries:

- **382 `ensure_standing` stays `-> Result`, though its false exit is an answer.** No original
  caller treats that exit as an error, and 34 ignore it. Changing the declaration to `-> bool` would
  refuse every file that declares 382 locally as `-> Result` next to `use std::globals::*;`. It is
  left as a follow-up with the renames.
- **286 keeps its parameter name `skill`**, so named calls still compile. Its comment says the
  parameter is a skill value (0..1000), not a skill index.

One compatibility edge: a file that has both `use std::globals::*;` and a local
`extern mod globals` declaring one of the 45 new trees with another ABI is now refused, because
the merge sees one tree called two ways. No file in the repository does this. The lift already puts
such trees in `more_globals`, and `duplicate_std_global` points at the local declaration. The fix
is to rename the local block.

The existing names stay so that the open pack and the pilots keep compiling. Several are close to
the original labels. Renaming them needs a migration and is left as a follow-up.

## Consequences

- The open pack builds byte-identical to the branch point.
- Track 7 follow-up: `lint.rs` does not yet know the new engine fields.
- Track 9 follow-up: lifted output names more global calls, and `global[20]` now lifts as
  `world.expansions`. It could also emit typed fields and flags.
- Out of scope for now:
  - a typed `bool + value` ABI for 282 and 333 (for example
    `#[ret(temp = 0)] -> Option<ObjectId>`);
  - `#[temp(0)]` parameters, for byte identity with originals that call only in temps mode.
