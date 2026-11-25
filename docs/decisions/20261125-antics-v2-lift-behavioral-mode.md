# `lift --v2 --behavioral`: opt-in behavioral checking for decompiled trees

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/decompile.rs`, the `lift` flags in
  `crates/sims-antics/src/bin/sims-antics.rs`
- **Track:** Track 9 (`lift --v2`)
- **Builds on:** [`20261123-verifier-behavioral-mode.md`](20261123-verifier-behavioral-mode.md)
  (the behavioral relation) and
  [`20261125-antics-v2-lift-corpus-optimizations.md`](20261125-antics-v2-lift-corpus-optimizations.md)
  (the strict baseline)

## Context

`lift --v2` keeps a tree idiomatic only when the default (strict) checker proves the
recompiled tree equivalent. The strict checker observes node numbers, because a fault or a
yield leaves the node number in the frame.

The v2 compiler numbers nodes in source order and cannot pin a node to a number. So a
tree whose structured form reorders or duplicates nodes never proves strictly, even when
every effect on the world is the same. The previous sweep traced about 8,600 trees to
node order alone and about 5,900 to duplicated shared tails.

`verify-equiv --behavioral` already defines a weaker, explicitly stated relation. It
leaves out node numbers and frame storage, and it observes everything else: primitives,
calls, untracked memory, exits, tracked cells, and whether and how a path faults.

## Decision 1: `--behavioral`, opt-in, for `lift --v2` only

`DecompileV2Options::behavioral` (CLI `lift --v2 --behavioral`) runs every tree check with
`LiveOptions::behavioral`. That is the same relation, with the same stated limits, as
`verify-equiv --behavioral`.

- **Default unchanged.** Without the flag, the lift is the strict one. Over the full
  corpus, the default output is byte-identical to the strict baseline.
- **`--behavioral` without `--v2` is an error.** The `.air` lift does not check its
  output; `verify-equiv --behavioral` checks `.air` lifts.
- **Only the code relation changes.** Every other part of the object-level contract
  (PR #444) runs as before:
  - header, sound and string gaps;
  - object gaps;
  - the propagation of unproven callees;
  - the fallback of an unproven tree to a lower level;
  - the exit status on a counterexample.

  A tree whose code proves behaviorally but has an object gap is still reported
  inconclusive.
- **Labelled output.** The source gets a header comment, and the stderr summary says
  `(behavioral, node numbers and frame storage not compared)`. Without validation nothing
  is checked, so neither appears.

  The header reads:

  ```
  // Verified in behavioral mode (`--behavioral`): node numbers and frame storage are not
  // compared, so each proof holds only for runs that start and end under one build
  // (docs/decisions/20261123-verifier-behavioral-mode.md).
  ```

**What a behavioral lift does not promise.** Behavioral proofs hold for runs that start and
end under one build. A frame saved by the original build and resumed by the rebuilt one
resumes at a node number that means something else. That includes a saved game whose
stack is inside such a tree. See the behavioral-mode decision for the stale-frame-word
assumption.

## Decision 2: in behavioral mode, the true branch goes first

Node order (previous decision) exists only to keep node numbers, which behavioral mode does
not compare. In behavioral mode the lift therefore starts from the structured lifter's
natural order, the true branch first, which needs no negated conditions. It also skips the
other-order retry: under a relation that ignores node numbers, the two orders can only
differ in node numbers.

Measured against node order with the retry, under the behavioral relation:
- idiomatic and raw counts are the same;
- lint falls from 190 to 124 findings, with 95 → 27 `negated_comparison`;
- the sweep ran a little faster (53 s against 60 s, one run each).

## Decision 3: a breakpoint that loops on itself allows `unbounded_loop_without_yield`

Behavioral mode writes some original debug traps idiomatically for the first time. A
breakpoint node whose true branch is itself comes out as `loop { breakpoint(..); }`. That
is the original's behaviour: it runs until the VM errors the object out.

The translator puts
`// antics-lint: allow(unbounded_loop_without_yield): the original's breakpoint loops on itself`
above exactly that shape: a `loop` whose whole body is one breakpoint. All 22 findings of
that rule were this shape. The strict corpus has no such loop, and its output is
unchanged.

## Results (full corpus, same install)

| Metric | Strict (default) | `--behavioral` |
| --- | --- | --- |
| Files lifted / timeouts | 1,191 / 0 | 1,191 / 0 |
| Trees | 26,206 | 26,206 |
| Identical / equivalent / inconclusive / omitted | 886 / 4,635 / 20,684 / 1 | 878 / 4,643 / 20,684 / 1 |
| DIFFERENT | 0 | 0 |
| Idiomatic trees | 8,472 (32.3%) | 25,296 (96.5%) |
| Raw-fallback trees | 17,733 (67.7%) | 909 (3.5%) |
| Inline `raw {}` statements | 7,608 | 79,174 |
| Lines | 978,879 | 1,199,538 |
| fmt-dirty files | 0 | 0 |
| Lint findings | 7 | 102 |

**Why the verdicts barely move.** Every one of the 20,684 inconclusive trees, in both
modes, is inconclusive for an object-level reason. Examples:
- the rebuilt object links another semi-global file;
- the tree may run code outside the file's private trees, or object callbacks;
- it calls an unproven tree;
- its label or callback registration differs.

None is a code-proof failure. A strict lift reaches a code proof by falling back to raw,
whose nodes match one for one. A behavioral lift reaches it at the idiomatic level, and
the object gap then applies the same way. So the mode changes how trees are written, not
what is claimed about them. The 8 trees that move from identical to equivalent are
written in the other branch order.

**Size.** Lines and inline raw statements grow because idiomatic trees now include the
trees with duplicated shared tails, and because statements without a v2 spelling stay
inline raw. Most of those statements are local-slot moves and stores.

**The 102 lint findings:**
- **72 `frame_headroom`.** Duplicated tails push the recompiled tree past 200 nodes where
  the original was below. The warning is accurate: the tree is close to the node limit.
- **27 `negated_comparison`.** These come from the original's negated compare nodes.
- **3 `collapsible_if_chain`.**

## Follow-ups

- **Readable locals.** About 9,000 one-statement raw blocks are moves between locals and
  `stack_obj`, temps, constants or data words. Naming every local a tree uses (as
  `#[slot(i)]` lets already do for the locals builtins address) would cover them. It
  changes strict output too, so it belongs in its own change.
- **Shared tails.** A v2 join construct would remove both the duplicated nodes behind
  `frame_headroom` and, in strict mode, the tail-duplication fallbacks.
