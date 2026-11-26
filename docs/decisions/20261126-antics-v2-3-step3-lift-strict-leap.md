# `lift --v2` v2.3 step 3: node pins, shared tails, named locals, `light`, `if let`

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/v2/decompile.rs`
  - new `tests/integration/v2_3_lift_strict_leap.rs`
- **Track:** Track 9 (`lift --v2`), `.antics` v2.3 step 3
- **Builds on:**
  - [`20261126-antics-v2-3-step2-node-order-and-shared-tails.md`](20261126-antics-v2-3-step2-node-order-and-shared-tails.md)
    (#491: `node_order`, `share_tails`, `light`);
  - [`20261126-antics-v2-3-step1-if-let-and-469-fixes.md`](20261126-antics-v2-3-step1-if-let-and-469-fixes.md)
    (#489: `if let Some(v) = f()`);
  - [`20261125-antics-v2-lift-wave3.md`](20261125-antics-v2-lift-wave3.md) (the polish byte gate).

## Context

Steps 1 and 2 gave the compiler a way to pin a tree's node numbers and to merge the tails
an original shares, plus `light` and `if let`. Strict mode compares node numbers across a
yield, so a structured lift whose nodes run in another order went raw. Behavioral mode had 46
`frame_headroom` trees, whose copied tails took them past 200 nodes. Separately, the #460
follow-up counted about 9,000 raw lines that name `local[N]`. Most of them sit in trees whose
extra idioms were dropped (`plain`) and in lifecycle blocks, which never named their locals.

As in earlier waves, nothing here changes what is claimed about a tree: every tree is still
rebuilt and proven, strict or `--behavioral`, and a tree a change breaks falls back.

## Decision 1: pin the nodes of an unproven tree

When a round leaves a tree unproven, the first step of its fallback chain is `pin_nodes`.
It tries `NodePin::matching(compiled, original)`:

1. `match_node_order(compiled, original)`. A non-identity order is written as
   `#[bhav(.., node_order = [..])]`.
2. Otherwise, the compiled nodes after `share_tails` (`widen` → `share_tails` → `narrow`, the
   compiler's own pass). When that merges something and the merged nodes match the original,
   the result is `share_tails`, plus `node_order` when that is not the identity.

The rules:

- **Keyed by text.** A pin is stored with the tree's text written without it. A pin is
  tried once per text, and a text that changes (another fallback step) is tried again.
- **Kind 4** (indexed trees) never gets a pin, since the compiler refuses one.
- **Refusals fall back.** A tree whose build refuses its pin (Decision 3 of step 2: an inline
  call into a wide frame) or that stays unproven with it loses the pin first (`drop_pin`),
  before any other fallback. Corpus sweeps drop 2 pins in behavioral mode and 7 in strict
  mode.
- **Behavioral mode proves trees without a pin, so it rarely writes one.** A pin there only
  comes from Decision 2.
- **Header patching.**
  - The header gains `, share_tails` and/or `, node_order = [..]` before its closing `)]`.
  - A lifecycle block written with the default `#[id(N)]` header gets its full
    `#[bhav(..)]` header (`lifecycle_header`) so that the pin has a place.

## Decision 2: share the tails of a tree past `frame_headroom`

After a round converges, `share_large` gives `share_tails` to each tree whose build holds
more than lint `FRAME_NODE_WARN` (200) nodes while its original holds no more, when merging
helps. It adds `node_order` when the merged nodes are the original's in another order. A tree
that is not proven with the pin loses it again (Decision 1).

- 44 of the 46 behavioral `frame_headroom` trees take it.
- The 2 left are a 201-node tree in two objects whose nodes do not merge. The 3 strict trees
  are unchanged.

**Not adopted: structuring past the node limit.** Review round 1 pointed out that a tree
whose structured body copies tails past 251 nodes never reaches `share_large`. The
structurer (`structure_lift.rs`) stops at 245 emitted nodes, and the tree is written as its
labeled nodes.

A prototype gave the v2 lift a budget of 1,004 nodes and added `share_tails` when the
build refused the tree:

- behavioral: 238 more idiomatic trees (96.6% → 97.5%);
- the lifted text grew by 7% (936k → 1,003k lines);
- inline raw lines rose from 76k to 115k;
- strict: one more idiomatic tree.

The new "idiomatic" trees are copied tails, mostly raw (one 43-node original became 151 raw
statements). A labeled tree of 43 nodes reads better, so the budget stays. A test pins it
(`a_body_past_the_node_limit_stays_labeled`).

## Decision 3: name the locals of plain trees and lifecycle blocks

`#[slot(i)] let slot_i: i16;` was written only when a tree kept its extra idioms. Trees
written `plain` and lifecycle blocks spelled every local as `raw { local[N] := .. }`.

- They now name the locals their statements address (scope 25, index below the tree's
  count), and the translator spells reads and writes as `slot_N`.
- **Fallback.** When a tree fails with names, `unname_locals` takes them away right after
  `drop_extras`. For a failure no tree is blamed for, `unname_all_locals` follows
  `drop_all_extras`.
- **Measured per tree.** Corpus sweeps take names away 239 times in behavioral mode and 8,030
  times in strict mode. Strict mode compares frame words, and a named local can move a
  temp.
- **More raw statements, fewer raw lines.** In 9 behavioral files, a raw block that names
  locals becomes v2 code with smaller raw blocks inside, so the raw statement count of
  those files rises while their raw lines fall (in one object, raw lines go from 536 to
  231). Lint `manual_raw_has_builtin` rises from 214 to 268 in 3 of those
  files, because a sprite flag cell that a plain tree does not spell now stands alone in its
  own raw block.

## Decision 4: `data[51]` is written `light`

`cell_spellings` rewrites `h.data[51]` to `h.light`, for a single-name holder (`self`,
`stack_obj`, a handle). Like the other cell spellings, the rewrite is kept only if the file's
bytes stay the same. The corpus has no `data[51]`, so a test covers it.

## Decision 5: `if let Some(v) = f()` and `let Some(v) = f() else { .. };`

`OPTION_CALLS` lists the `std::globals` trees declared `-> Option<T>` with
`#[ret(temp = 0)]`: `take_elapsed_periods` (`periods`) and `find_companion` (`companion`). A
unit test checks the list against `std/globals.antics`. After the flag-method groups,
`polish` runs `if_let_spellings`:

- **`if globals::f(..) { .. }`** becomes `if let Some(name) = globals::f(..) { .. }`, and the
  `temp[0]` reads in the then block become `name`.
- **`if !globals::f(..) { .. }` with no else** becomes
  `let Some(name) = globals::f(..) else { .. };`. The reads after it, up to the end of the
  enclosing block, become `name`.
- **Where reads stop:**
  - any write to the temp: an assignment, `flag_set` / `flag_clear`, a method call, `&` or
    `mut`;
  - any call, since a call may change `temp[0]`;
  - any `raw { }` block.
- **When a statement is skipped:**
  - the name is already spelled somewhere in the file;
  - a group with no read is not offered.
- **Byte gate.** Each statement is one group. Groups go through `keep_joint`, which keeps a
  set only if the whole file's bytes are unchanged.
- **Corpus result.**
  - behavioral: 13 `if let` and 14 `let .. else`;
  - strict: none.

  In strict mode, the binding's frame word changes the bytes.

## Decision 6: review round 1

- **Not adopted: structuring past the node limit** (Decision 2).
- **`flag_set` / `flag_clear` and methods stop the reads.** `temp[0] flag_set 1` is a write.
  The byte gate refused the whole group before, so the read before the write lost its name.
  A test covers it.
- **Test coverage.** A test pins the wide-inline-call case:
  - the structurer does not reorder around it;
  - the tree is proven with no pin.

  The `drop_pin` and `unname_locals` paths are exercised by the corpus counts above.

## Results

The base install, 1,191 objects and 26,206 trees. Before is `lift-v2-2-final-*`; after is
`lift-v2-3-*`.

| | behavioral before | behavioral after | strict before | strict after |
|---|---|---|---|---|
| idiomatic trees | 25,308 (96.6%) | 25,308 (96.6%) | 8,503 (32.4%) | 9,523 (36.3%) |
| identical / equivalent | 870 / 4,651 | 870 / 4,651 | 884 / 4,637 | 1,113 / 4,408 |
| raw-fallback trees | 897 | 897 | 17,702 | 16,682 |
| zero-raw objects | 250 | 264 | 50 | 56 |
| inline raw statements | 14,553 | 13,346 | 3,058 | 3,441 |
| inline raw lines | 78,913 | 76,438 | 5,915 | 8,017 |
| `node_order` / `share_tails` pins | 0 / 0 | 0 / 44 | 0 / 0 | 938 / 297 |
| lint `frame_headroom` | 46 | 2 | 3 | 3 |
| lint `manual_raw_has_builtin` | 214 | 268 | 3 | 3 |

Notes on the table:

- **Strict raw statements and lines grow** because 1,020 trees that were raw are now
  idiomatic, and they bring their raw blocks with them.
- **Strict alarm trees.** The 2 strict alarm trees from #469/#473
  are idiomatic again in strict mode. That comes from #489 Decision 4(c).

Invariants, both modes:

- 0 DIFFERENT;
- 0 errors or timeouts;
- 0 `fmt --check` dirty files;
- by `cmpbase`: no object has fewer idiomatic or proven trees than before;
- the 5 style rules stay at 0;
- `collapsible_if_chain` stays at 2 and `stale_lint_allow` at 1 in behavioral mode.

Sweep time is 112 s behavioral (was 122 s) and 136 s strict (was 119 s).

## Consequences

- **Track 7 (lint):** `manual_raw_has_builtin` flags sprite-flag cells (scope 27) left alone
  in a raw block of a plain tree. The lift does not spell them there, because the tree's
  extra idioms are off.
- **Follow-ups:**
  - strict `if let` would need the binding to reuse the temp's frame word;
  - per-tree raw-cell spellings in plain trees;
  - `structure_lift.rs` could emit shared tails directly (a `goto`-free join) instead of
    copying them, which would let the trees in Decision 2's measurement lift without the
    bloat.
