# `lift --v2` v2.4: raw builtin blocks, shared-tail joins, the strict-mode gap

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/structure_lift.rs`
  - `crates/sims-antics/src/v2/decompile.rs`
  - `tests/integration/v2_3_lift_strict_leap.rs`
- **Track:** Track 9 (`lift --v2`), `.antics` v2.4
- **Builds on:**
  - [`20261126-antics-v2-3-step3-lift-strict-leap.md`](20261126-antics-v2-3-step3-lift-strict-leap.md)
    (#493: node pins, shared tails, named locals). Its follow-ups are items 1 and 2 here.
  - [`20261125-antics-v2-lift-wave3.md`](20261125-antics-v2-lift-wave3.md) (the polish byte gate).

## Context

v2.3 step 3 left three things open:

1. **Behavioral `manual_raw_has_builtin` rose from 214 to 268.** Plain trees named their
   locals, and a sprite-flag cell (scope 27) was left alone in its own `raw { }` block.
2. **The structurer copies every tail that both branches of an `if` share into each
   branch.** That bloats the text, and it pushes trees past the 245-node budget, where they
   are written as labeled nodes.
3. **Most strict-mode trees are still raw (16,682 of 26,206), and nobody had measured why.**

As before, nothing changes what is claimed about a tree. Every tree is rebuilt and checked,
strict or `--behavioral`. A tree that a change breaks falls back.

## Decision 1: raw blocks that hold only builtins are written as builtins

`polish` gains a group after the flag-method groups, `raw_builtin_spellings`. It applies to
a `raw { }` block whose every statement is one of these:

| raw statement | written |
|---|---|
| `scope(27, n) := v` (`v` a temp or a number) | `stack_obj.sprite_flags[temp[n]] = v;` |
| `temp[k] := scope(27, n)` | `temp[k] = stack_obj.sprite_flags[temp[n]];` |
| `remove_object(me \| stack_obj, ..)` | `me.remove(..)` / `stack_obj.remove(..)` |
| `clear_motive_changes() pad(i: v, ..)` | `clear_motive_changes(padI: v, ..);` |

Scope 27 is the stack object's sprite flag indexed by `temp[n]`.

The details:

- **`remove`.** It is written in the method form that lint `prefer_method_syntax` asks for.
  The two known flag bits are named (`no_yield`, `skip_resets`). Other bits stay as `flags: N`.
- **A block with any other statement stays raw,** as does a target other than `me` or
  `stack_obj`.
- **Byte gate.** Each block is one group, kept by `keep_joint` only if the file's bytes stay
  the same. The groups get their own budget of builds (`POLISH_RAW_BUILDS`, 24).

**Result:** `manual_raw_has_builtin` goes from 268 to 2 in behavioral mode and from 3 to
1 in strict mode. In each mode, what remains is a block that also does something else.

## Decision 2: the structurer joins shared tails

At an `if`/`else` with no merge and two real branches, `shared_join` looks for a node `J`
that both branches reach, where every other way out of the branches is a terminal:

- `return`;
- the tree's error or false exits.

When it finds one, the structurer writes the following, and the shared code after `J` is
written once:

```rust
while true {      // formatted `loop {`
    if cond { ..; break; } else { ..; break; }
}
<the code from J on>
```

The v2 compiler lowers `while true { ..; break; }` with no extra nodes; its own `if`
lowering (`cond.rs`) uses the same idiom.

**Rules:**

- **Barriers.** A branch that reaches any of these other than through `J` has no join:
  - `cur` again, or `stop_at`;
  - an active node;
  - the enclosing loop's header or exit.

  Inside the `while true`, `break` and `continue` would name the wrong loop.
- **Choice of `J`.** The candidate with the most nodes after it wins, the lowest number on
  a tie.
- **Merges inside a join:**
  - a merge must lie in the join's region or be `J` itself;
  - merges use the post-dominators of the enclosing real loop's DAG (`LoopCtx::dag`).
- **Two terminators.** When both branches end at once and one of them is `break;`, the
  structurer writes `if cond { <term> } <term>`, not an `if`/`else`.
- **Joined or copied: whichever builds fewer nodes.** The structurer writes the body both
  ways and keeps the joined one only when it builds fewer nodes. A join that saves no node
  adds a loop for nothing.
- **Node budget unchanged.** The 245-node budget stays. A body that copies its tails past
  it now lifts joined, with no `share_tails` pin. `a_body_past_the_node_limit_joins_its_tails`
  (6 arms × 45 stores) and the 5-arm `frame_headroom` test both now lift that way.

## Decision 3: the other tail spelling is a per-tree fallback

The lifter keeps, for each structured tree, its lift with the other tail spelling
(`TreeModel::other_tails`) in both branch orders. The copied lifts' two orders are made on
their own: two joined lifts can be one text where the copied ones are two.

- **Which goes first.** Whichever of joined and copied has fewer lines is written first;
  on a tie, the structurer's choice (fewer nodes) goes first.
- **When the other is tried.** `Plan::copy_tails` swaps it in once, before the tree goes
  raw, at three points:
  - in the unproven chain, after `other_order`;
  - in the `.v2` build-error chain, before the line-level fallback;
  - in `air_fallback`, before `lower_tree`.

  It restores the tree's extra idioms (`retry_extras`). In the unproven chain, strict mode
  then gives the other branch order another try (`other_order`). The build-error chains
  try only the first order, as they always have for `other_order`. A build error comes
  from a statement, which both orders write.

**Why a fallback.** Without it, 13 strict trees regressed: a join builds nodes in an order
that the original does not run. With it, no object loses an idiomatic or proven tree.

**Not adopted: copied first in strict mode.** That would keep the v2.3 text wherever it
proves. Strict sweeps of the three orders:

| first | lines | inline raw lines | identical |
|---|---|---|---|
| copied | 966,644 | 8,325 | 1,113 |
| fewer nodes (the structurer's choice) | 966,369 | 7,658 | 1,116 |
| fewer lines (adopted) | 966,192 | 7,662 | 1,116 |

Copied-first loses wherever the joined text is shorter and proves too, because that tree
passes through `pin_nodes` and the other fallbacks before it gets back to the join.

**Pins.** Joins share tails in the text, so fewer trees need a pin:

- strict `share_tails` pins go from 297 to 32;
- behavioral `share_tails` pins go from 44 to 20.

Strict `node_order` pins rise from 938 to 988.

## Decision 4: why strict-mode trees stay raw (read-only census)

A temporary probe was used for the census and is not committed. It recorded, for each tree
that strict mode leaves raw, the last failure of its idiomatic text. It also compared the
original with the v2 compiler's nodes for that text. The census covers 16,638 raw trees,
from a v2.4 build. That build wrote the structurer's choice first; it leaves the same 16,638
trees raw as the final build:

| why the idiomatic text did not prove | trees |
|---|---|
| same node count, other CFG | 9,818 |
| node count matches only after `share_tails` | 4,467 |
| more nodes than the original (copied code) | 1,571 |
| still more nodes after `share_tails` | 6 |
| the structurer gives up (unstructurable CFG) | 560 |
| same CFG and operands, other numbering (no pin applied) | 179 |
| fewer nodes than the original | 28 |
| same CFG, operands differ | 4 |
| same CFG and operands | 4 |
| `.air` build error | 1 |

**The main finding: the "other CFG" bucket is almost entirely dead false targets.**

- With a walk that ignores the false target of a statement that cannot fail, 9,669 of the
  9,818 trees have the original's CFG in another numbering.
- Of the rest, 137 also have operand differences, mostly primitive operands, and only 6
  have a truly different CFG.

**Why.** The original compiler writes `false: error` (0xFD) for a statement that cannot
fail; the v2 compiler writes `false: next`. Most of the trees in the four node-count
buckets compile fewer `false: error` exits than their original:

| bucket | trees with fewer `false: error` exits |
|---|---|
| same node count | 9,818 of 10,005 |
| matches after `share_tails` | 3,949 of 4,473 |
| more nodes | 1,407 of 1,571 |
| fewer nodes | 28 of 28 |

`NodePin::matching` needs equal nodes, so these trees never get a pin.

**Checker verdicts.** The strict checker's verdicts on these trees are about 8,895 "path
budget" and about 7,145 "undecided". A pin that matched would make most of them
Identical.

**Follow-up for Track 8 (lowering).** Add a way to spell a statement's dead false target
`error`: per statement, or per tree as the original compiler's default. With that, the lift
can write what the original holds, and most of the 9,818 + 4,467 trees could pin.

## Decision 5: review rounds

Codex reviewed two rounds.

- **Round 1.**
  - **Both copied branch orders are made on their own.** `copied_tails` used to make the
    second copied order only when the joined lift had two orders, so a tree could go raw
    without trying the copied order that runs the true branch first.
  - **Raw statements left in the report after polish.** `DecompiledTree::raw_statements`
    still counted raw statements that the polish had written as builtins. `polish` now
    returns, per tree, the statements it took out of raw blocks. `tree_id_before`
    attributes each rewrite to the last tree header before it, and the report subtracts
    them. Behavioral inline raw statements: 12,597 before the fix, 12,323 after.
- **Round 2.**
  - **Build errors try the first branch order only.** That is qualified in Decision 3
    rather than changed.
  - **`merge_arms` double count.** A pre-existing overcount of raw statements shared by
    both arms (`merge_arms`, #471) is left as a follow-up.
  - **Stale count.** The strict raw count in this ADR is updated.
  - **Checked and clean:** join targeting, nested loops, negation and the node budget.

## Results

The base install: 1,191 objects, 26,206 trees. Before is `lift-v2-3-*`, after is
`lift-v2-4-*`.

| | behavioral before | behavioral after | strict before | strict after |
|---|---|---|---|---|
| idiomatic trees | 25,308 (96.6%) | 25,580 (97.6%) | 9,523 (36.3%) | 9,567 (36.5%) |
| identical / equivalent | 870 / 4,651 | 889 / 4,632 | 1,113 / 4,408 | 1,116 / 4,405 |
| raw-fallback trees | 897 | 625 | 16,682 | 16,638 |
| zero-raw objects | 264 | 267 | 56 | 56 |
| inline raw statements | 13,347 | 12,323 | 3,441 | 3,517 |
| inline raw lines | 76,439 | 57,515 | 8,017 | 7,662 |
| total lines | 936,369 | 912,746 | 965,908 | 966,192 |
| `node_order` / `share_tails` pins | 0 / 44 | 0 / 20 | 938 / 297 | 988 / 32 |
| lint `manual_raw_has_builtin` | 268 | 2 | 3 | 1 |
| lint `frame_headroom` | 2 | 2 | 3 | 3 |
| lint `collapsible_if_chain` | 2 | 4 | 0 | 0 |

Invariants, both modes:

- 0 DIFFERENT;
- 0 errors or timeouts;
- 0 `fmt --check` dirty files;
- by `cmpbase`: no object has fewer idiomatic or proven trees than before;
- the 5 style rules stay at 0.

**Strict total lines rise by 284 (0.03%).** Per file:

- **38 files gain 44 idiomatic trees: +497 lines.** A tree that was raw is now written as
  v2 code, which is longer than its labeled nodes.
- **Every other file that changed: −213 lines net.** That is 209 files.

The 3 files that grow most without a new idiomatic tree (+97 lines each) share one
tree. Its joined text now proves with a `share_tails` pin, so the copied text that v2.3
proved, which is shorter in the final branch order, is never tried. The lift does not
compare lines across branch orders. Strict inline raw statements rise by 76, but inline raw lines
fall by 355: the new idiomatic trees bring small raw blocks.

**Behavioral:** 272 more idiomatic trees, 23,623 fewer lines, and 18,924 fewer inline raw
lines. The raw statements of 104 files rise: a raw tree that becomes idiomatic brings its
raw blocks along. For example, one object goes from 1 raw tree to 0 and from 1 inline
statement to 91.

Sweeps take 111 to 150 s per mode across runs, against 104 to 136 s before. The machine was
loaded during these runs. The lift now structures each tree both ways (joined and copied),
twice more for its other tail spelling.

## Consequences

- **Track 7 (lint):**
  - `collapsible_if_chain` goes from 2 to 4 in behavioral mode, on two trees that were raw
    before.
  - The `manual_raw_has_builtin` findings left are mixed raw blocks.
- **Track 8 (lowering):** the dead-false spelling in Decision 4.
- **Follow-ups:**
  - the 179 renumbered trees with the original's CFG and operands, which no pin reaches yet;
  - the 560 unstructurable trees;
  - `DecompiledTree::raw_statements` counts the raw statements the lifter wrote, minus those
    the polish writes as builtins (review round 1). It still counts twice a raw statement
    that both arms of an `if` share and that `merge_arms` (#471) writes once after it.
    Counting from the final tree bodies would fix that, but it changes what the sweeps
    have measured since v2.1.
