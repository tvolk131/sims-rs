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
  stack object. A local destination is declared like any other local the tree names.
- **`find_best_object(kind[, flags: f])` as a condition.**
  - It is used with no origin and outside a cursor loop.
  - The kind is named from the function table.
  - A search with no kind stays raw.
- **Definition cells.** Read on the definition's handle:
  - definition cells 9 and 11 are `x.object_type` and `x.sub_index`;
  - a function-flag cell tested against 0 is `x.has_function[N]` or `!x.has_function[N]`,
    for the functions v2 names (0 to 255);
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

## Round order

Two fallback chains change.

- **A round that does not compile**, blamed on a tree, tries in order:
  1. `grow_locals` (an `.air` locals error only);
  2. `add_frame`;
  3. `keep_temps_raw`;
  4. `unmerge`;
  5. then the existing steps: builtin locals only, drop inherit, drop extras, number raw
     BCONs, then lower.
- **An unproven tree** tries `keep_temps_raw`, then `fix_locals`, then `unmerge`, before the
  existing steps.

## What stays raw, and why

- **Calls across contexts** whose callee could not be written inherit (unchanged).
- **`run_tree_by_name`.** Not attempted in this wave.
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
| Idiomatic / raw-fallback trees | 8,494 / 17,711 | 8,511 / 17,694 | 25,308 / 897 | 25,308 / 897 |
| Inline `raw {}` statements | 3,636 | 3,201 | 27,796 | 18,552 |
| Lines inside inline `raw {}` blocks | 7,184 | 6,281 | 153,806 | 102,220 |
| `raw {}` blocks (inline and whole-tree) | 19,887 | 19,533 | 22,721 | 13,131 |
| `if !..return error` | 1,313 | 581 | 33,497 | 5,263 |
| `?` | 199 | 2,034 | 1,218 | 32,072 |
| Lines | 971,842 | 965,773 | 1,137,208 | 950,286 |
| fmt-dirty files | 0 | 0 | 0 | 0 |
| Lint findings | 12 | 10 | 100 | 51 |
| `frame_headroom` | 3 | 3 | 70 | 46 |
| `negated_comparison` | 9 | 7 | 27 | 3 |
| `untyped_engine_field` | 0 | 0 | 0 | 0 |
| Wall time, 10 workers | 108 s | 85 s | 70 s | 73 s |

- **Verdicts.** Every count is unchanged. Per file, no tree loses a proof.
- **Idiomatic trees.** No file has fewer idiomatic trees in either mode. 17 strict files have
  more.
- **Remaining `if !..return error` in behavioral mode.** About 1,000 test a call. These sit
  inside an animation event handler or a raw block, or their tree dropped its extras. The
  rest test a comparison, not a call.

## Follow-ups

- **`run_tree_by_name`.** Spell it in the lift.
- **Node headroom.** Shared suffixes across `match` arms and loop exits, the 46 remaining
  `frame_headroom` findings, need a v2 form such as a local helper or a labeled block to be
  written once. (Track 8, lowering.)
- **Strict temps calls.** Strict mode could name a temps call when v2 can pass temps without
  copies: `call N(temps)` with every argument the temp of the same index. This needs a v2
  lowering rule. (Track 8.)
