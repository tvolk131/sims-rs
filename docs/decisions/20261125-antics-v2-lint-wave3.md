# `.antics` v2 lint, wave 3: typed `std` sync and ergonomics rules

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`, `sims-antics lint`; test-only lines in
  `v2/decompile.rs`
- **Track:** Track 7 (`.antics` v2.1 Wave 2, Stream 4b)
- **Builds on:** [20261125-antics-v2-lint-wave2.md](20261125-antics-v2-lint-wave2.md),
  [20261125-antics-v2-typed-std.md](20261125-antics-v2-typed-std.md)

## Context

The typed `std` foundation (#468) added these:

- **`std::flags` and `std::engine`.** Named bits and values: `ObjectFlags::InUse`,
  `RefreshMode::RecountRoom` and the like.
- **Typed engine words.** One table in `lower/tree.rs` serves `self`, Sims, typed handles and
  `stack_obj`. It names 13 more words: `facing`, `container`, `wall_adjacency`, `room_impact`,
  `broken`, `weight`, `support_strength`, `flags2`, `value`, `buy_flags`, `category`,
  `sim.gender_kind` and `sim.date_partner`.
- **`std::globals`** grew to 62 trees.

`untyped_engine_field` knew only the older names, and only on `self` and Sims. Three more rules
were asked for:

- names for engine numbers;
- `?` for a `Result` call whose failure is only passed on;
- method spellings of builtins.

## Decision 1: a suggestion stands only if it builds the same bytes

The engine words differ in their rules:

- some are strict enums (`self.facing = 3` is an error);
- some are `ObjectId`s;
- some are read-only;
- a state of the same name wins over the word;
- a `use` may be missing, aliased or shadowed.

Copying all of lowering's typing into the lint would drift. Instead, each new finding carries its
source edit, and the lint builds the file again with the edit applied. The finding stands only
when every object of the file builds to the **same `.iff` bytes**. The checked finding therefore:

- compiles;
- changes nothing the game runs;
- keeps the lift's contract: `engine_field_fixes` applies suggestions blindly.

How the builds run:

- **Groups.** A file's pending findings are built together in groups, and a failing group is
  split in halves.
- **What shares a group.** Findings of one statement, and findings whose edits overlap (a
  `refresh` constant and the call's method form), never share a group. A group build therefore
  never accepts an edit that builds only alongside another, such as two `ObjectId` cells compared
  with each other. Each accepted finding builds on its own, given the rest of its group.
- **Budget.** A file gets at most 48 builds. On the corpus, the most any file needed was 3.
- **Imports.** A `use` line a finding needs is inserted at the top for the build. A test showed
  that line numbers do not reach the `.iff`.
- **Where checks run.** Only a file that builds is checked. A pack file builds against its pack
  under its own file name, as `check` does.
- **Order.** Checked findings join the others before the `stale_lint_allow` pass and
  suppression. An `allow` of a new rule therefore works and is not stale.

The existing static `untyped_engine_field` path (`SELF_FIELDS`, `SIM_FIELDS`) is unchanged and is
not re-checked. Every one of its 1,442 corpus findings is still reported.

## Decision 2: `untyped_engine_field` follows the typed word table

Past the static tables, the rule proposes a name for a `data`, `person` or `slot` cell with a
literal index. The name depends on the root:

- **`self`** (where it is the object): the object words.
- **A Sim name:** the person words, `gender_kind` and `date_partner`, then the object words.
- **`stack_obj` and handle locals:** `hand`, `room`, the person words (`age`, `priority`,
  `visitor_kind`, `kind`, ...), then the object words.

Two filters run before a proposal is built:

- **Read-only names** (`container`, `hand`, `room`, `visitor_kind`) are skipped where the cell is
  written. `ObjectId` names are proposed only where compared with `==` / `!=` or assigned whole,
  as before.
- **The enum words** `facing` (`Direction`) and `gender_kind` (`PersonKind`) are proposed only
  under `as i16`. Their other uses never type-check alone: on the corpus none of these proposals
  built. Pairs such as `self.data[1] = stack_obj.data[1]` would need both rewritten.

A state of the same name keeps the cell. `world.expansions` (`global[20]`) was already covered,
because the rule reads the parser's `WORLD_GLOBALS`. `duplicate_std_global` reads
`std/globals.antics` itself, so it covers all 62 trees. Tests pin both.

## Decision 3: `untyped_engine_constant`

The rule reports a number literal that stands for a named `std::flags` bit or `std::engine` value.
It applies in these places:

| Place | Type |
| --- | --- |
| `w flag_set N`, `w flag_clear N`, `w flag_test N` on a typed flag word (by name or by number) | the word's `std::flags` type: `Height`, `WallAdjacency`, `ObjectFlags`, `WallPlacement`, `ObjectFlags2`, `Placement`, `BuyFlags` |
| the same on `global[20]` / `world.expansions` | `Expansions` |
| the `category:` of `can_afford`, `debit_funds`, `credit_funds` (call or method form) | `BudgetCategory` |
| `refresh(target:, mode:)`, and the mode of `x.refresh(m)` | `RefreshTarget`, `RefreshMode` |

Rules for the suggestion:

- **No guessing.** Only literals are reported, and only when the type names the value. A named
  constant is left alone, and so is a bit without a name.
- **Names come from the `std` source.** They are read from `std/flags.antics` and
  `std/engine.antics`, so the lint cannot disagree with them.
- **Spelling the type.**
  - Prelude types (`Height`, `Placement`) need no `use`.
  - A glob or named `use std::flags` / `std::engine` import gives the type's spelling, alias
    included.
  - Otherwise the help asks for `use std::<module>::<Type>;`, and the check build adds it.
  - When the file declares or imports another item of that name, nothing is reported.

## Decision 4: `prefer_question_mark`

The rule reports `if !f(..) { return error; }`, or `if f(..) {} else { return error; }`, when `f`
surely returns `Result`. That means one of:

- a tree of the file set declared `-> Result`;
- an `extern mod` function declared `-> Result`;
- a `std::globals` tree declared `-> Result`, called as `globals::f` in a file that imports
  `std::globals` and does not declare its own `globals` module entry for it.

The suggestion is `f(..)?;`. Nothing is reported in these cases:

- `f` returns `bool` (a `false` answer is not an error);
- the branch holds anything else;
- the `if` is an `else if`;
- a comment sits in the statement (the rewrite would drop it);
- the code is inside `raw { }`;
- a binding named `error` is in scope.

## Decision 5: `prefer_method_syntax`

A builtin call used as a statement is reported when its target has a method form:

| Call | Method form |
| --- | --- |
| `play_sound(s, x)` / `play_sound(s, target: x)` | `x.play_sound(s)` |
| `stop_all_sounds(x)` | `x.stop_all_sounds()` |
| `refresh(target: x, mode: m)` | `x.refresh(m)` |
| `push_interaction(object: x, ..)` | `x.push_interaction(..)` |
| `remove_object(x, ..)` | `x.remove(..)` |

For `refresh`, target `0` / `RefreshTarget::Me` is the frame's `me`. The suggestion spells it:

- `self` in an object's own tree;
- the Sim's name in a Sim's body;
- `me` in an inline helper, which runs in its caller's frame.

Target `1` / `StackObject` is `stack_obj`.

Nothing is reported for a user function of the same name, a call used as a value, a non-identifier
target, or a call holding a comment.

## Results

- **Open pack:** 0 findings with `--deny-warnings`. Its build is unchanged: the change touches only
  the lint.
- **Corpus** (`cache/scratch/corpus-v2-behavioral`, 1,191 lifted files): 0 errors. Lint time is
  2.6 s against 1.8 s on main, at 8 threads.

  | rule | main | this change |
  | --- | ---: | ---: |
  | `untyped_engine_field` | 1,442 | 5,971 |
  | `untyped_engine_constant` | – | 4,976 |
  | `prefer_question_mark` | – | 2,290 |
  | `prefer_method_syntax` | – | 3,884 |
  | other rules | 1,808 | 1,808 (unchanged) |

  Every proposal that reached a build stood.

## Consequences

- **Lift (Track 9).**
  - The lift applies `untyped_engine_field` suggestions, so lifted objects now name `stack_obj`,
    handle and new object words. One lift unit test that pinned `stack_obj.data[28]` now expects
    `stack_obj.support_strength`.
  - Lifted text does not yet use the three new spellings. The lift tests' lint-clean assertion
    ignores those three rules for now; the coordinator approved this test-only edit in
    `decompile.rs`.
  - **Follow-up for Track 9 Stream 3b:** emit the spellings, then drop the filter.
- **Cost.** A file with pending findings builds at least once more inside `lint`. `stale_lint_allow`
  and the new rules need a building file.
