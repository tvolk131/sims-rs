# `.antics` v2.2 lint polish: Sim dot names, flag methods, and `negated_comparison` before an exit

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`; new `tests/integration/v2_2_lint_polish.rs`;
  `docs/roadmap.md` (Batch 19).
  No change to `v2/decompile.rs`, `v2/lower/*`, `v2/parser.rs` or `v2/std/*`.
- **Track:** Track 7 (lint), `.antics` v2.2 polish wave, Step 1
- **Builds on:** [`20261125-antics-v2-lint-wave3.md`](20261125-antics-v2-lint-wave3.md)
  (byte-gated `untyped_engine_field` and `prefer_method_syntax`),
  [`20261125-antics-v2-lint-panic-and-bcon-paths.md`](20261125-antics-v2-lint-panic-and-bcon-paths.md)
  (#478)

## Context

- v2 names a Sim's skills, personality traits and motives with dots: `sim.skill.cooking`,
  `sim.personality.neat`, `sim.motive.fun`. `untyped_engine_field` did not propose them for
  the numbered cells (`sim.person[10]`, `sim.person[7]`, `sim.motive[15]`). The lift writes
  the numbered form at about 1,900 sites.
- Typed `std::flags` words have methods: `.set(F)`, `.clear(F)`, `.contains(F)`. The lift
  still writes the operators `flag_set`, `flag_clear` and `flag_test` (about 4,000 sites
  with a typed constant).
- The wave asked to widen `negated_comparison`. A successor that is `return true;`,
  `return false;`, `panic!();` or `return error;` would count as leaving the compare's
  result unobserved, as a quiet store does today.

## Decision 1: dot names for a Sim's numbered skills, traits and motives

`untyped_engine_field` proposes these for a Sim (`sim`, or `me` in a Sim's tree),
`stack_obj`, and a local holding a handle:

| cell | dot name |
| --- | --- |
| `person[2]`, `[3]`, `[5]`, `[6]`, `[7]` | `personality.nice`, `.active`, `.playful`, `.outgoing`, `.neat` |
| `person[10]`, `[11]`, `[12]`, `[15]`, `[17]`, `[18]` | `skill.cooking`, `.charisma`, `.mechanical`, `.creativity`, `.body`, `.logic` |
| `motive[3]`, `[5]`–`[9]`, `[11]`, `[13]`–`[15]` | `motive.mood`, `.energy`, `.comfort`, `.hunger`, `.hygiene`, `.bladder`, `.alertness`, `.room`, `.social`, `.fun` |

- The indices come from lowering's own tables: `skill_by_name`, `motive_by_name`
  (`parser.rs`) and `personality_trait` (`lower/prim_table.rs`).
- The new `SIM_GROUP_WORDS` table goes through `trial_engine_field`. Like the other trial
  words, a proposal stands only when the rewritten file builds the same bytes, alone and
  together with the file's other `untyped_engine_field` rewrites. That holds for reads, for
  writes (`=`, `+=`), for `stack_obj`, and for handle locals.
- A typed handle's object is no Sim: lowering refuses `h.skill.x` there, so that proposal
  falls.
- As before, only a literal index counts (`sim.person[COOKING]` stays), and a cell with a
  comment inside it is left alone.
- The rewrite replaces the text `root.cell[N]`, blanks allowed (`stack_obj.data [41]`), so
  a receiver spelled otherwise (`(sim).person[10]`) gets no finding.
- The lift applies `untyped_engine_field` suggestions, so its output now uses these names
  (see "Track 9" below).

## Decision 2: flag methods, under `prefer_method_syntax`

`place flag_set T::F;` is `place.set(T::F);`, `flag_clear` is `.clear(..)`, and
`place flag_test T::F` is `place.contains(T::F)`, also inside `!( .. )` and other
expressions. They go under the existing `prefer_method_syntax` rule, not a new
`prefer_flag_methods`. The rule is already "this builtin form has a method form that builds
the same nodes", it is already byte-gated, and users who allow it expect both. The rule's
summary now names flag operators.

Reported only in an object's body whose template, if any, is in the same file, so that
every state and type its body sees is known. A template's body (its objects may be in other
files), a module-level helper (inlined into its callers), and an object built from another
file's template get no flag-method finding. Then both of these must hold:
- **The receiver is a named object flag word that lowering types as bitflags:** `flags`,
  `flags2`, `buy_flags`, `placement`, `allowed_heights` or `wall_adjacency`, of `self`, `me`,
  `stack_obj` or a Sim.
  - Not a cell by number (`self.data[8]`); `untyped_engine_field` names it first.
  - Not a state of the same name in the object or its template on `self` (or on `me`
    outside a Sim's body; in a Sim's body `me` is the Sim), nor a field of a handle local,
    whose object may declare such a state. Another object's or template's state does not
    count.
  - Not `wall_placement`. Lowering keeps it an `i16` that takes `WallPlacement` constants
    (`lower/tree.rs` `OBJECT_DATA_WORDS`), so `.set()` does not type-check there.
  - Not `world.expansions`, which lowering reads as `global[20]`, an `i16`.
- **The operand is a constant of the word's own `std::flags` type, by a name that means
  that type in the file.** Every import of it counts, by name or alias (`ObjectFlags` and
  `Of` together). The type's own name also counts when a `std::flags` glob or the prelude
  (`Placement`, `Height`) brings it in, unless another module's glob may bind it. An
  explicit import wins over any glob.
  - A raw bit (`flag_set 6`) gets `untyped_engine_constant` first. The method form follows
    once the operand is typed.
  - A prelude type's alias counts too (`use std::flags::{Placement as P}`, then
    `P::Water`).
  - Not a name another import or a file item takes, nor a type of that name declared in
    the object or its template. A file type `ObjectFlags` leaves an alias `Of` its
    meaning.
  - Not a constant of another type, or any other expression.

Known limits, all quiet where a finding would be right (never the reverse):
- a type name that only a glob or the prelude brings in, in a file that also has a glob
  of another module, whatever the order of the globs;
- a prelude name (`Placement`, `Height`) that a file item also takes, such as a scalar
  `const Placement`;
- `me` in a tree that runs in both contexts, in an object with a state of that name.

The rewrite covers the receiver, the operator and the operand, so it applies only when that
text is exactly `receiver op T::F`. A comment there would be lost, so none is reported. As
with every `prefer_method_syntax` finding, it stands only when the rewritten file builds
the same bytes.

## Trial budgets: the new rewrites come last

The lint tries at most 48 rewrite groups per file, and the joint `untyped_engine_field`
check at most 16 builds. The dot names and the flag methods are *secondary* proposals: they
are grouped and built only after every other proposal, and never share a group or a joint
build with one. So a file full of them keeps every finding main reports (a raw bit's
constant, `stack_obj.value`, `self.id`); only the secondary ones fall when the budget runs
out.

## Decision 3: `negated_comparison` stays quiet before an exit

The widening is unsound, so the rule is unchanged. A compare's own result is observed when
the tree ends right after it:

- **Lowering** writes `if !(self.level > 0) { return false; }` as the compare node
  `my.attr[2] > 0`, whose false branch goes straight to exit 255 (`return false`; 254 is `return true`, 253 the error exit). No node sits between the
  compare and the exit.
- **The VM**, on exit 254 or 255, pops the frame before it writes the exit value. In
  `sims-vm/src/interpreter.rs`, `branch` calls `pop_frame`, which passes the old last result
  to `release_use`. `release_use` hands it to `end_action(last_result)`
  (`object_uses.rs:176`) when the person's last object use ends.
  - That old last result is the compare node's own branch result.
  - The flipped compare (`self.level <= 0`) reaches the same exit on its opposite result.
  - So the person's last action records the opposite result.
- **On an error exit** (`panic!()`, `return error;`) the last result stays. Play-mode
  recovery rebuilds the object's frame with it (`visits.rs:280`), and a person's reset copies
  it into the last-action flags (`people.rs:1297`).
- **A bare `return;`** lowers to `return true;` and is the same case.
- **The verifier agrees.** With default settings, `verify-equiv` on the two builds says
  INCONCLUSIVE: "last branch result at exit differs: A false vs B true". The trees prove
  equivalent only with `--ignore-last-result`, an assumption the CLI prints. Before
  `panic!()` they stay INCONCLUSIVE even with `--ignore-fault-last-result`.

The tests pin all three points:
- not reported before each exit, on either branch;
- the flipped build does not prove equivalent;
- before `return true/false`, the flipped build proves equivalent once the last result is
  ignored;
- the quiet-store case is still reported, and proves equivalent with the last result
  observed.

## Consequences

- `tests/integration/v2_2_lint_polish.rs` has 29 tests, positive and negative, for each
  decision and for the trial budgets. Every rewrite the tests apply builds identical bytes.
- **Open pack:** lints clean with `--deny-warnings`, passes `fmt --check`, and builds
  byte-identical to main (`611795a30`). It uses neither form.
- **Lift corpus** (`cache/track9/lift-panic-discard-beh`, 1,191 files from main after #477):
  - 0 errors;
  - every finding main reports is still reported, and no other rule gains one;
  - `untyped_engine_field`: +1,692 (1,395 skills, 297 traits; roots `sim` 793, `me` 644,
    `stack_obj` 255). The corpus has no numbered motive outside `raw` blocks;
  - `prefer_method_syntax`: +3,979 flag methods in 821 files (`flags` 2,564,
    `buy_flags` 595, `placement` 526, `flags2` 178, `wall_adjacency` 116);
  - lint time for the corpus: 2.2 s → 3.1 s wall (8 threads), from the extra trial
    builds.
- **Track 9 (the lift):** five lift tests need Track 9's parallel PR, which writes the
  dot names and the flag methods:
  - four assert that lifted text lints clean, and now see the flag operators the lift
    still writes:
    - `v2::decompile::tests::bit_operations_and_sim_slots_leave_raw`;
    - `v2::decompile::tests::calls_and_flag_tests_are_returned_whole`;
    - `v2_lift_rust_polish::motives_constants_and_imports_take_their_paths`;
    - `v2_lift_wave3::typed_constants_bring_their_imports_and_method_forms_replace_target_arguments`;
  - `v2_lift_pr_h_idioms::a_tree_both_contexts_call_is_written_inherit` expects
    `me.person[3] = 1;`. The lift applies `untyped_engine_field` and now writes
    `me.personality.active = 1;`.
  - The lift reads a `prefer_method_syntax` finding only as `name(..)`, so it skips the
    flag findings rather than applying them wrongly.
