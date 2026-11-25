# `lift --v2` wave 2: `?`, joined arms, #466 spellings, named temps calls

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/decompile.rs`
- **Track:** Track 9 (`lift --v2`), `.antics` v2.1 Wave 1 Stream 3a
- **Builds on:**
  - [`20261125-antics-v2-lift-inline-raw-cleanup.md`](20261125-antics-v2-lift-inline-raw-cleanup.md)
    (named locals, typed engine fields, what stayed raw);
  - [`20261125-antics-v2-pr-i3-expansion-gaps.md`](20261125-antics-v2-pr-i3-expansion-gaps.md)
    (the v2 forms this change starts writing).

## Context

After #463, lifted code still spelled most failure exits out:
- `if !f(..) { return error; }`, more than 33,000 times in behavioral mode;
- if-chains repeated the same arm body under two tests, or the same last statements in
  every arm;
- #466 added v2 forms that the lifter did not yet write, such as `.remove()`, `set_to_next`
  into any cell, `find_best_object` as a condition, and the definition and job cells;
- several primitives kept whole statements raw:
  - temps calls to trees with literal parameters;
  - `create_object`, push-interaction with an icon local, relationship values,
    motive-change primitives;
  - dialogs used as conditions.

Nothing here changes what is claimed about a tree. Every new spelling is checked like any
other: the lift recompiles the file and proves each tree (strict or `--behavioral`), and a
tree that a spelling breaks falls back as before. The #444 soundness contract is unchanged.

## Decision 1: `?`

- `if !f(..) { return error; }` is written `f(..)?;`.
- With a plain return on the other arm, it is written `f(..)?; return X;`:
  - `if f(..) { return X; } else { return error; }`;
  - `if !f(..) { return error; } else { return X; }`.
- In each form the test is one call or primitive spelled as a call. It is the same node, and
  its false edge goes to the error exit.
- It is not used:
  - inside an animation event handler, where `return` is v2's to place;
  - when any statement it would cover is forced raw.
- It is an extra idiom (`Translator::extra`), so a tree it breaks is rewritten without it.

## Decision 2: joined arms and shared tails

After an if-chain is written, its arms are compared as text:

- **Joined arms.** Adjacent arms with the same body become one arm whose test is the arms'
  tests joined with `||`.
- **Shared tails.** When the chain has a final `else`, the statements that end every arm move
  after the chain. Comments just before a statement (such as a lint `allow`) move with it, and
  a comment does not count as a statement. Three conditions apply:
  - every arm keeps at least one statement;
  - no arm declares a `let` or `const` of its own;
  - no test is a `let` condition.

v2 lowers a joined arm once. The original has one copy per arm.

- **Behavioral mode** does not compare node numbers, so the one copy is proven against the
  original's two.
- **Strict mode** keeps a merge only where the merged tree is still proven.

A tree that a merge breaks is rewritten without merges (`Plan::unmerged`) before its other
extras are dropped.

## Decision 3: the #466 spellings

- **Remove.**
  - The remove-object primitive on another object is `x.remove(..)`:
    - the receiver is `me`, `sim` or `self` for target 0;
    - for target 1 it is the cursor or `stack_obj`;
    - the options are `no_yield: true`, `skip_resets: true`, or `flags: N`.
  - It also works as a condition.
  - Removing the tree's own object keeps `destroy_self`.
- **`set_to_next(.., dest: x)`.** `x` may be a local, a parameter or a temp, not only the
  stack object. A local destination of an `if` or a statement is declared like a builtin's
  local. A loop's is not: written as a `for`, v2 gives the cursor its own words.
- **`find_best_object(kind[, flags: f])` as a condition.**
  - It is used with no origin and outside a cursor loop.
  - The kind is named from the function table.
  - A search with no kind stays raw.
- **Definition cells.** Read on the definition's handle:
  - definition cells 9 and 11 are `x.object_type` and `x.sub_index`;
  - a function-flag cell tested `!= 0` or `== 0` is `x.has_function[N]` or
    `!x.has_function[N]`, for the functions v2 names (0 to 255). A `> 0` test keeps its
    scope spelling: v2 lowers the name to `!= 0`, which differs from `> 0` for a negative
    cell, and the checker cannot rule one out;
  - the handle is `self` in a Sim tree that never moves `stack_obj`, and `stack_obj`
    otherwise;
  - none is written inside a cursor loop.
- **Job cells** are `job_field(F)`.

## Decision 4: fewer raw primitives

Each new spelling below is an extra idiom.

- **Temps calls to trees with literal parameters (behavioral mode only).**
  - `call N(temps)` is written `f(temp[0], .., temp[k])`.
  - v2 copies each temp through a `let` and back, so the tree may need more frame locals
    than its header pins. Behavioral mode does not compare frame storage, so the header gains
    them when a round asks:
    - the count is read from v2's "requires at least N" message (`Plan::grow_locals`);
    - the `.air` build stops at the first tree short of locals. So a round that fails on one
      also builds the file once with every header's count left out (`Model::locals_needed`),
      which finds every other tree that is short. Each takes its next step in that same
      round, instead of costing one round each: it grows if it can, and otherwise steps
      down the usual fallback chain (`Plan::air_fallback`). Leaving a tree out (`omit`)
      still waits for the tree's own round, so it gets the build's own message.
      - The full corpus is byte-identical to one tree per round.
      - The slowest object went from about 250 compile rounds (36 s) to 30 (5.5 s), and
        `main` takes 14 s on it;
    - it is capped at the larger of the original count and the `frame_headroom` lint
      threshold (12);
    - a tree with no arguments and no locals never grows. The behavioral checker compares
      that frame shape (`args == 0` with `locals == 0`).
  - Fallbacks:
    - A tree that the copies break returns to raw temps calls (`Plan::keep_temps_raw`),
      with its original locals count. One cause is dialog text that reads frame locals
      through `$Local` tokens.
    - An unproven tree whose locals grew gets its original count back once
      (`Plan::fix_locals`) before anything else is lowered. Without that step, extra locals
      would cost the tree its idiomatic level instead of one raw statement.
  - Strict mode keeps the raw call, since the copies are other nodes.
- **`create_object(guid, mode: m, flags: f[, local: slot_N], dest: stack_obj)`.**
  - It is written as a statement or a condition outside a cursor loop.
  - It is the bare primitive, with no cursor save.
  - The GUID is named like other GUID operands (a `GUID_xxxxxxxx` const), so lint
    `magic_guid_literal` does not fire.
- **`push_interaction(.., icon: slot_N)`** when the icon comes from a local
  (`flags & 3 == 1`).
- **Relationship values.** The value may be:
  - a named local;
  - a temp or a parameter;
  - a literal, when flag 4 says it is one.
- **Motive changes.**
  - `clear_motive_changes()`.
  - `set_motive_change(Motive::Name, delta, max)` for a motive below 16, by number when the
    motive has no name.
- **A dialog as a condition**: `if dialog(..) {`. As a statement whose false exit is an error,
  it is written `dialog(..)?;`.

## Decision 5: a broken new spelling costs only the new spellings

Every spelling above goes through one switch (`Translator::wave2`, under the older
`Translator::extra`). A tree that a round with them does not compile or prove first loses
them alone (`Plan::drop_wave2`):
- it is written without them;
- it keeps the original's locals, and its header may not grow again.

Only then does it lose its older extras (`drop_extras`). That step drops every idiom the
tree had before this change, so one failing new spelling would otherwise cost the tree more
than it gained.

Without the switch, a full-corpus review found 28 behavioral files and 9 strict files with
more raw statements than `main`.

Two more cases count as wave 2:
- **Unwritten spellings.** A tree whose `#[slot]` pins include a local that only a wave 2
  spelling names is marked, even when that spelling is not written. `drop_wave2` can then
  remove the pin.
- **Unblamed failures.** A failure blamed on no tree drops wave 2 in every tree
  (`drop_all_wave2`) before every tree's extras.

## Round order

Two fallback chains change.

- **A round that does not compile**, blamed on a tree, tries in order:
  1. `grow_locals` (an `.air` locals error only; every other tree short of locals takes
     its next step in the same round, see Decision 4);
  2. `add_frame`;
  3. `keep_temps_raw`;
  4. `unmerge`;
  5. `drop_wave2`;
  6. then the existing steps: builtin locals only, drop inherit, drop extras, number raw
     BCONs, then lower.

  Trying builtin locals only before `grow_locals` was measured and rejected. It does avoid
  unused `#[slot]` pins next to a `for` loop's own words. But it lowers temps-call trees
  further: one file went from 59 to 210 raw statements.
- **An unproven tree** tries in order:
  1. `keep_temps_raw`;
  2. `fix_locals`;
  3. `unmerge`;
  4. `drop_wave2`;
  5. then the existing steps.

## What stays raw, and why

- **Calls across contexts** whose callee could not be written inherit (unchanged).
- **`run_tree_by_name`.** `main` already writes it as a statement, and it is not among
  the 25 most common raw statement shapes.
- **`scope(N, i)` operands** for scopes without a v2 field (unchanged).
- **Large trees.** 46 behavioral `frame_headroom` findings remain, down from 70.
  - All are node headroom: a lifted tree that compiles to more than 200 of the 251 nodes a
    tree holds.
  - None is new.
  - The growth comes from structuring: the original shares a node suffix between arms of a
    `match` or a loop, and structured code writes that suffix once per arm.
  - Shared-tail factoring removes the cases where the suffix ends every arm of an if-chain.
    The rest need a jump into a shared suffix, which structured v2 has no form for.

## Results (full corpus, same install)

- **Before** is `main` (d7ae63738); **after** is this change.
- Both cover 1,192 candidate entries in each mode: 1,191 objects lifted and 0 timeouts.
- Lint is counted with the after build on both outputs.
- In the table, `if !..return error` counts `if !<test> { return error; }` statements, and
  `?` counts `?;` lines.

| Metric | Strict before | Strict after | `--behavioral` before | `--behavioral` after |
| --- | --- | --- | --- | --- |
| Identical / equivalent / inconclusive / omitted | 884 / 4,637 / 20,684 / 1 | 884 / 4,637 / 20,684 / 1 | 870 / 4,651 / 20,684 / 1 | 870 / 4,651 / 20,684 / 1 |
| DIFFERENT | 0 | 0 | 0 | 0 |
| Idiomatic / raw-fallback trees | 8,494 / 17,711 | 8,505 / 17,700 | 25,308 / 897 | 25,308 / 897 |
| Inline `raw {}` statements | 3,636 | 3,188 | 27,796 | 18,253 |
| Lines inside inline `raw {}` blocks | 7,184 | 6,263 | 153,806 | 104,214 |
| `raw {}` blocks (inline and whole-tree) | 19,887 | 19,534 | 22,721 | 13,167 |
| `if !..return error` | 1,313 | 583 | 33,497 | 5,905 |
| `?` | 199 | 2,028 | 1,218 | 31,286 |
| Lines | 971,842 | 965,811 | 1,137,208 | 953,299 |
| fmt-dirty files | 0 | 0 | 0 | 0 |
| Lint findings | 12 | 10 | 100 | 52 |
| `frame_headroom` | 3 | 3 | 70 | 46 |
| `negated_comparison` | 9 | 7 | 27 | 3 |
| `collapsible_if_chain` | 0 | 0 | 3 | 3 |
| `untyped_engine_field` | 0 | 0 | 0 | 0 |
| Wall time, 10 workers | 108 s | 103 s | 70 s | 70 s |
| Slowest object | 52 s | 35 s | 26 s | 11 s |

- **Verdicts.** Every count is unchanged. Per file, no tree loses a proof.
- **Idiomatic trees.** No file has fewer idiomatic trees in either mode.
- **Raw statements per file.**
  - Behavioral: one file has one more raw statement than `main`, and it gains `?` in many
    places.
  - Strict: four files each turn a whole raw tree idiomatic at the cost of two inline raw
    statements.
  - Strict: one tree's shared-tail merge proves it in the Sim context, where `main` fell
    back to the object context. Its calls to object trees then cross contexts and stay raw
    (+2). It is proven, as before.
- **Remaining `if !..return error` in behavioral mode.** About 1,800 test a call. These sit
  inside an animation event handler or a raw block, or their tree dropped the new spellings
  or its extras. The rest test a comparison, not a call.
- **Time.** The host's load average was between 10 and 150 during these runs, and above
  60 for the strict after run.

## Follow-ups

- **Node headroom.** Shared suffixes across `match` arms and loop exits, the 46 remaining
  `frame_headroom` findings, need a v2 form such as a local helper or a labeled block to be
  written once. (Track 8, lowering.)
- **Strict temps calls.** Strict mode could name a temps call when v2 can pass temps without
  copies: `call N(temps)` with every argument the temp of the same index. This needs a v2
  lowering rule. (Track 8.)
