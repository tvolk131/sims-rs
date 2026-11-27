# `lift --v2` v2.5 step 1B: locals read in place, `if let` in short trees, the strict-gap census

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/v2/decompile.rs`
  - new `tests/integration/v2_5_lift_locals.rs`
- **Track:** Track 9 (`lift --v2`), `.antics` v2.5 step 1B
- **Builds on:**
  - [`20261126-antics-v2-4-lift-dead-false-and-v24-features.md`](20261126-antics-v2-4-lift-dead-false-and-v24-features.md)
    (#499: `dead_false = error` pins, 4,961 strict raw-fallback trees, 6 behavioral
    `prefer_if_let` left);
  - [`20261126-antics-v2-5-compiler.md`](20261126-antics-v2-5-compiler.md) (#503, step 1A:
    `keep_layout`, extern frame widths, fixed-frame option bindings, `get_up`).

As before, nothing changes what is claimed about a tree. Every tree is rebuilt and checked,
strict or `--behavioral`, and a tree that a change breaks falls back. The corpus results
below are measured against `origin/main` with #503: its lift gives the same metrics as
`lift-v2-4-final-*` (the 382 calls are spelled `get_up()` now), so every difference is this
step's.

## Context

A local that the lift leaves by number (a tree whose locals it could not name) is read and
written in `raw { }` lines. v2 has `local[N]` for such a word, but it is read-only, and a
read of it keeps the tree's layout (`keep_layout`), as a raw block does. Over the v2.4 lifts,
the raw lines of idiomatic trees that name a local were 7,839 strict and 6,309 behavioral.

The 6 behavioral `prefer_if_let` findings left by #499 were statements whose `let..else`
binding builds the same bytes, but which the polish never tried: the `if let` search has a
budget of 24 builds, and a file's statements in trees short of a frame word for the binding
spent it first.

## Decision 1: reads of an unnamed local are `local[N]`, after the lift converges

Once the fallback loop has converged (every tree checked, no step left to take), one more
pass writes each statement that only reads a local left by number as v2 code:
`temp[0] = local[0];`, `stack_obj = local[1];`, `if local[2] <= Tuning::e0 {`.

- **It changes no node.** The pass regenerates the whole object with the reads in place and
  keeps that text only when the object builds the bytes the converged text built. So no
  verdict, pin or count of proven trees can change. When the bytes differ, each tree that
  read a local in place and whose nodes changed keeps its reads raw, and the pass builds
  again (at most `LOCAL_READ_BUILDS` = 4 builds). A build error names its tree the same way.
  A failure that names any other tree keeps the converged text.
- **Pins carry over.** A tree pinned in the converged round (`node_order`, `share_tails`,
  `dead_false`) keeps its pin with the new text. The pin is otherwise tied to the text it was
  computed for. The byte check covers it.
- **Writes stay raw.** v2's `local[N]` is read-only; a write needs Track 8 (see
  Consequences).

**Why after convergence and not as a fallback step.** Two earlier designs were measured over
the corpus and dropped:

- *A step in the fallback loop* (read in place from the start, step a tree back to raw reads
  when it fails). Every extra failing round also steps the tree's callers down, since a caller
  is proven only with its callees. Even with no fallback at all, reading in place from the
  start cost 11 strict idiomatic trees and 1 behavioral error, since a read can change nodes.
- *Raw layout anchors* (keep one raw block per tree to pin the layout and read everything
  else in place). Clutter for the reader, slower, and 276 of 307 anchored files still failed.

The post-pass cannot lose a tree: it either keeps the converged text or takes text that
builds the same bytes.

**Review round 1:** a failed build used to blame one tree per retry (`Iterator::any` stopped
at the first insertion), so four refusing trees could spend the budget before a fifth tree's
reads were tried. Every blamed tree now keeps its reads raw at once
(`every_tree_a_failed_read_build_blames_keeps_its_reads_raw`).

## Decision 2: `if let` statements in short trees are tried last

A binding takes a frame word, which a tree whose header fixes its locals may not have. Before
the `if let` search, one build of every candidate at once, with the headers' locals counts
left out, names each tree that would need more locals than it declares
(`short_of_locals`). The candidates in those trees are searched after the others:

- the first search gets `POLISH_IF_LET_BUILDS` (24), as before;
- the second gets the builds the first left, and at least `POLISH_IF_LET_SHORT_BUILDS` (8).
  It builds every candidate the first search kept, too: the byte gate is always over every
  kept group.

When no candidate is in a short tree, or all are, the search is the single search over every
candidate that it was before (review round 1: the deferred group used to get 8 builds even
then). When one candidate keeps the census build from building, its parts are built breadth
first, at most `POLISH_IF_LET_CENSUS_BUILDS` (6) builds, so the parts that build are counted
before a failing one is split further. A part that still fails names no tree (review round
2: the census used to name none and the starving came back;
`a_statement_that_breaks_the_census_build_leaves_the_rest_counted`).

**Result:** the 6 behavioral `prefer_if_let` findings are now `let..else` statements
(`let..else` 23 → 29). The strict lift has no such site.

## Decision 3: the 2 strict alarm trees stay raw

The 2 strict trees in question are one tree, copied into two objects of one family. Its strict
lift keeps `raw { if param[0] == 1 { my.data[34] := 0; } else { my.data[34] := 0; } }`, and
the tree is proven with it. A native `if` builds one node fewer (12 bytes less in the FAR):
v2's `merge_duplicate_tails` joins the two identical arm tails. No pin can bring the node
back (`share_tails` merges, it does not split). Behavioral already writes the native `if`.
Unblocking the strict lift needs a Track 8 spelling or attribute that keeps the arms as
written.

## Decision 4: the strict-gap census

The 4,961 strict raw-fallback trees, classified by a probe build of this branch (not
committed). For each tree, the probe records the last round in which the tree was written
as v2 code and was not proven. It compares that rebuild with the original:

- the node count, with always-true nodes' `false` targets ignored;
- whether merging tails would give the original's count;
- the graph up to renumbering;
- the operands;
- the dead-`false` targets;
- whether some node of the original is reached by no branch;
- whether `match_tree_pin` finds a pin.

Each tree is in the first class that fits, from the top of the table down:

| class | trees | with unreachable nodes | mixed dead-false | a pin matches |
|---|---|---|---|---|
| same CFG and operands, renumbered | 1,933 | 1,920 | 0 | 9 |
| partial `share_tails` | 1,128 | 1,124 | 0 | 0 |
| copied code (more nodes) | 1,084 | 503 | 0 | 0 |
| unstructurable CFG | 560 | 216 | 20 | 0 |
| operand diffs (same CFG) | 142 | 11 | 0 | 0 |
| mixed dead-false | 85 | 59 | 85 | 0 |
| rebuild has fewer nodes | 28 | 3 | 0 | 0 |
| build error | 1 | 0 | 0 | 0 |
| **total** | **4,961** | **3,836** | **105** | **9** |

The class test order is: unstructurable, build error, mixed dead-false, copied code, partial
`share_tails`, fewer nodes, then the same-count classes. A tree counts as mixed dead-false
when some always-true node of the original goes to `error` on `false` and another does not:
105 trees, against the brief's estimate of 196. The unstructurable column counts its trees
too (from the original alone).

**Trees with unreachable nodes.** 3,836 of the 4,961 trees (3,620 of the 4,400 that reached a check) have a node in the original that no branch reaches. `match_tree_pin` only matches such a tree
as it stands (`match_bases`: "a tree with a node no branch reaches matches only as it
stands"). The v2 backend never emits a node no branch reaches, so these trees cannot be
pinned. The largest class, 1,920 trees whose rebuild is the original's reachable graph with
the same operands, needs nothing else. A way to keep the original's unreachable nodes, for
example a pin that lists them, would make each of them a candidate for an exact pin.

**The other columns:**

- *same CFG and operands, renumbered*: the rebuild is the original's graph with other node
  numbers. With every node reachable, a `node_order` pin would rebuild it; 9 had one, which
  the checker did not prove.
- *partial `share_tails`*: the rebuild copies some tails that the original shares, and
  merging every alike tail gives the original's node count but no pin matches.
- *copied code*: the rebuild has more nodes, and merging tails does not give the original's
  count. The original shares code that v2 writes twice in a way that `share_tails` does not
  undo.
- *operand diffs*: the same graph with different operands in some nodes.
- *mixed dead-false*: the original's always-true nodes go to `error` on `false` in some
  places and elsewhere in others, which neither `dead_false` setting writes.
- *unstructurable CFG*: the lift found no structured form; the tree is written as labeled
  nodes from the start.
- *build error*: the tree's last v2 round did not build (a tree short of locals).

## Remaining raw lines that name a local

Raw lines of idiomatic trees (labeled raw-fallback trees not counted) that name `local[N]`:

| kind | strict v2.4 | strict v2.5 | behavioral v2.4 | behavioral v2.5 |
|---|---|---|---|---|
| write (`local[N] := ..`, `+=`, `dec_gt`, ..) | 3,722 | 3,722 | 2,762 | 2,762 |
| primitive operand (`dest: local[N]`, `value: local[N]`) | 556 | 556 | 385 | 385 |
| read statement, block also writes a local | 1,387 | 1,090 | 1,169 | 1,031 |
| read statement, block writes no local | 699 | 511 | 727 | 460 |
| condition, block also writes a local | 987 | 896 | 883 | 794 |
| condition, block writes no local | 488 | 305 | 383 | 274 |
| **total** | **7,839** | **7,080** | **6,309** | **5,706** |

The reads left are mostly in raw blocks that are raw for another reason: a write of a local,
or a primitive with a local operand, in the same block (an `if` whose arm writes is one
block). The rest are in objects where the in-place reads did not build the same bytes within
the pass's 4 builds, or in trees whose nodes they change. Native `local[N]` reads: 743 in 113
strict files, 591 in 110 behavioral files.

## Results

The base install: 1,191 objects, 26,206 trees. The baseline is `origin/main` (#503), whose
metrics equal `lift-v2-4-final-*`; final is `lift-v2-5-step1-*`.

| | strict v2.4 | strict v2.5 | behavioral v2.4 | behavioral v2.5 |
|---|---|---|---|---|
| idiomatic trees | 21,244 (81.1%) | 21,244 (81.1%) | 25,590 (97.6%) | 25,590 (97.6%) |
| identical / equivalent | 4,234 / 3,310 | 4,234 / 3,310 | 1,143 / 6,401 | 1,143 / 6,401 |
| raw-fallback trees | 4,961 | 4,961 | 615 | 615 |
| inline raw statements | 25,087 | 24,509 (−578) | 12,326 | 11,896 (−430) |
| inline raw lines | 100,604 | 98,820 (−1,784) | 57,502 | 56,118 (−1,384) |
| inline raw blocks | 14,962 | 14,927 | 9,409 | 9,318 |
| total lines | 966,953 | 966,720 | 912,940 | 912,571 |
| lint findings | 26 | 26 | 9 | 3 |

Invariants, both modes:

- 0 DIFFERENT;
- 0 errors or timeouts;
- 0 `fmt --check` dirty files;
- by `cmpbase` against `lift-v2-4-final-*` and the `origin/main` lift: no object has fewer
  idiomatic or proven trees.

**8 files per mode gain raw statements while losing raw lines.** A raw `if` whose condition
reads a local becomes a native `if` with a raw block in each arm, so one block of N lines
becomes several smaller ones, each counted. In every such file the inline raw lines fall or
stay equal (for example 595 → 542 and 557 → 459 strict, 130 → 32 behavioral).

**Remaining lint:**

| mode | rule | count | cause |
|---|---|---|---|
| strict | `manual_raw_has_builtin` | 25 | layout anchors; `keep_layout` (#503) replaces them in step 2B |
| strict | `frame_headroom` | 1 | unchanged from #499 |
| behavioral | `manual_raw_has_builtin` | 2 | mixed blocks (unchanged) |
| behavioral | `collapsible_if_chain` | 1 | its `match` spelling fails the byte gate (unchanged) |

`prefer_if_let` goes from 6 to 0. No lint is suppressed.

## Consequences

- **Track 8 (lowering):**
  - a writable `local[N]` that keeps the layout. The 3,722 strict and 2,762 behavioral raw
    writes, and the reads that share their blocks, wait for it;
  - a way to keep an original's unreachable nodes (3,836 of the 4,961 strict gap trees);
  - a spelling that keeps identical `if` arms unmerged (Decision 3).
- **Track 9, step 2B:** write `keep_layout` in place of the 25 strict layout anchors, and
  move the `semi_globals_wide` trees to their library block (#503 Decisions 1 and 2).
- **Track 7 (lint):** nothing new.
