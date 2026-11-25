# `lift --v2`: corpus-driven readability and speed fixes

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/decompile.rs`, `crates/sims-antics/src/structure_lift.rs`
  (one additive entry point), `crates/sims-antics/src/v2/lint.rs` (two thresholds made shared;
  lifecycle findings mapped by pinned id)
- **Track:** Track 9 (`lift --v2`)
- **Follows:** `20261125-antics-v2-rust-like-syntax.md`

## Context

A full-corpus `lift --v2` sweep after the Rust-like syntax (PR I-1) gave a baseline:

- 1,192 candidate object files: 1,190 lifted, 1 non-object, 1 timeout.
- 26,142 trees: 875 identical, 4,605 equivalent, 20,661 inconclusive, 1 omitted, 0 DIFFERENT.
- 7,698 idiomatic trees (29.4%) and 18,443 raw-fallback trees, with 7,477 inline `raw {}`
  statements.
- 1,112 files carrying their own `extern mod globals` block, and none using `std::globals`.
- 268 lint findings: 118 `raw_bare_call_false_exit`, 93 `stack_obj_clobber_after_self`,
  35 `frame_headroom`, 22 `redundant_bool_return`.
- Interaction (TTAB) comments repeated once per OBJD on multi-OBJD files.
- 0 fmt-dirty files.

None of the fixes below may weaken the code-only object-level contract (PR #444). Every lifted
tree is still proven by the default checker, where node numbers are observable, or it falls
back a level. Nothing here touches what is compared.

## Diagnosis: why trees fall back to raw

The first unproven reason was recorded for every raw-fallback tree, along with its original and
recompiled node counts.

| Cause | Trees (approx.) | Fixed here |
| --- | --- | --- |
| Same node count, different node order: an opcode runs at another node number, which a fault or yield leaves on the stack | 8,600 | partly (branch order) |
| More nodes: the structured lifter duplicates a shared tail that v2, without `goto`, cannot join | 5,900 | no |
| Fewer nodes | 2,200 | no |
| A standalone call was written as a v2 bare call, which ignores the result (the AIR original exits false) | 415 | yes |
| `.is_type(..)` on an `i16` data word: a compile failure, one recompile round each | 745 | yes (47 left) |

Most of the "loop not bounded by literals; cut-point bisimulation failed" and "path explosion"
reasons reduce to the first two rows once their cut points are inspected.

**The timeout.** One expansion-pack controller object has 64 large trees. At the idiomatic
level each spends 3–6 s in the path checker before ending inconclusive, mostly by exhausting
the 250,000-pair budget. The whole file took about 55 s alone, and over 60 s under the sweep's
parallel load.

## Decision 1: lift branches in node order

`structure_lift::lift_bhav_structured_in_node_order` is a new, opt-in entry point; the existing
entry points are unchanged. It writes a two-way `if`/`else` with the branch whose first node is
lower first, negating the condition when that is the false branch. v2 numbers nodes in source
order, so the recompiled tree keeps more of the original numbering.

Node order is a heuristic, not a guarantee: the old (true-branch-first) order can be the one
that matches. When a structured tree's two lifts differ, the model keeps both. Before an
unproven tree drops a level, it is retried once at that level in the other order, so a tree
proven by either order stays idiomatic. A retry never skips the proof.

## Decision 2: a standalone call keeps its false exit

An AIR statement `call N(..);` exits the tree false when the call returns false. The
translator writes it as `if !f() { return false; }`, the same node. It previously wrote `f();`,
which ignores the result and so never proved. Combined forms:

- `call; return true;` is written `return f();`: the same node with the same exits.
- In raw blocks, a bare `call N(..);` line is written `if !call N(..) { return false; }`, the
  form `raw_bare_call_false_exit` asks for.

## Decision 3: `return <condition>` for calls, negations and flag tests

`if c { return true; } else { return false; }` already became `return c;` for comparisons and
for primitives in one polarity. Each of these is one node with both exits on returns (checked
against the AIR each produces), so the fold now also covers:

- tree calls, except `std::globals` trees declared `-> Result`, which are no `bool` to return;
- the negated polarity (`return !f();`);
- `flag_test`.

An `else if` arm that only returns its condition is written `else { return c; }`.

## Decision 4: `use std::globals::*;`

A library tree `std::globals` declares, called with the same number of literal arguments as
its declaration has parameters (temps-passed calls match zero parameters), is named from
there. Its call spelling becomes `globals::<std name>(..)`. Every other library tree stays in
a local block named `more_globals`, because a local `globals` would hide the import.

Calls are only ever written in condition position, where both `-> bool` and `-> Result`
declarations type-check, so the call ABI and arity never change. The std table is read from
`std/globals.antics` itself.

## Decision 5: smaller lint fixes

- **`stack_obj_clobber_after_self`.** In a Sim-context tree that assigns `stack_obj`, the stack
  object's cells are written `stack_obj.attr[i]` / `stack_obj.slot[i]` (the same operands), not
  `self.*`.
- **`.is_type`.** A receiver of data word 11 is written `<handle>.id` and the Sim's data word 2
  `sim.container`, the handle-typed spellings v2 accepts.
- **`frame_headroom`.** A tree whose original already has more nodes or locals than the lint's
  thresholds gets `// antics-lint: allow(frame_headroom)`. A faithful lift keeps the original's
  size. The thresholds are now shared constants in `lint.rs`. The comment goes after both
  header forms (`#[bhav(..)]` and a lifecycle block's `#[id(N)]`). The lint now maps `init` and
  `main` findings by their pinned id, as it already did for `fn` trees, so the allow applies.
- **TTAB comments.** Identical interaction tables are written once. With several distinct tables,
  each gets a `TTAB <ids>:` header line.

## Decision 6: a smaller pair budget for `lift`

`lift` runs the checker with at most 50,000 pairs of paths with different outputs, instead of
250,000; the other bounds keep their defaults.

- Running out leaves a tree unproven, so it lowers a level; it never proves one.
- Over the full corpus, the 50,000 and 250,000 sweeps produced byte-identical output for every
  file except the one that timed out, which now lifts in about 10 s.

## Results (full corpus, same install)

| Metric | Before | After |
| --- | --- | --- |
| Files lifted / timeouts | 1,190 / 1 | 1,191 / 0 |
| Trees | 26,142 | 26,206 (the formerly timed-out file's 64) |
| Identical / equivalent / inconclusive / omitted | 875 / 4,605 / 20,661 / 1 | 886 / 4,635 / 20,684 / 1 |
| DIFFERENT | 0 | 0 |
| Idiomatic trees | 7,698 (29.4%) | 8,472 (32.3%) |
| Raw-fallback trees | 18,443 | 17,733 |
| Inline `raw {}` statements | 7,477 | 7,608 |
| Lines | 981,310 | 978,879 |
| Files with `use std::globals::*;` / own `extern mod globals` | 0 / 1,112 | 1,101 / 12 (862 also keep `more_globals`) |
| Interaction comment lines | 6,626 | 5,291 |
| Lint findings | 268 | 7 |
| fmt-dirty files | 0 | 0 |

On the same 1,190 files, excluding the newly lifted one:

| Metric | Before | After |
| --- | --- | --- |
| Idiomatic trees | 7,698 (29.4%) | 8,459 (32.4%) |
| Raw-fallback trees | 18,443 | 17,682 |
| Inline raw statements | 7,477 | 7,587 |
| Lines | 981,310 | 976,652 |
| Proven trees (identical + equivalent) | 5,480 | 5,480 |

The 7 remaining lint findings:

- 4 `negated_comparison`. `!(a == b)` is the original's negated compare node; `a != b` is a
  different operator, and swapping the branches back would break node order.
- 3 `frame_headroom`, on ordinary `fn` trees: each recompiles to 201 nodes from an original
  below the 200-node threshold, so no allow is written.

## Follow-ups

- **Node pinning in the v2 compiler (Track 8)** is the main lever left: about 8,600 trees fail
  only on node order. That needs a way to place a statement's node at a given number, or a
  checker relation that treats a fault's node number as unobservable. The latter would weaken
  the #444 contract, so it is not proposed.
- **Joining shared tails** (about 5,900 trees) needs a v2 construct that compiles to one shared
  node sequence: a labelled block or `goto`-free join.
- **Context-dependent primitives in `context = inherit` trees** (sounds, `destroy_self`) still
  cost one recompile round each before the statement lowers. A static check would only save
  time.
