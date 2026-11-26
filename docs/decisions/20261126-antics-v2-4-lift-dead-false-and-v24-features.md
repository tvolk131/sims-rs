# `lift --v2` v2.4 step 3: `dead_false = error` pins, semi-global links, the v2.4 spellings

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/v2/decompile.rs`
  - `tests/integration/v2_4_lift_finale.rs`
  - `tests/integration/v2_lift_pr_h_idioms.rs`
- **Track:** Track 9 (`lift --v2`), `.antics` v2.4
- **Builds on:**
  - [`20261126-antics-v2-4-lift-scope27-and-joins.md`](20261126-antics-v2-4-lift-scope27-and-joins.md)
    (#496: raw builtin blocks, joins, the strict-gap census). Its census found that most
    strict raw trees differ from their rebuild only in dead `false: error` targets.
  - #498 (`dead_false = error`, `TreePin`, `match_tree_pin`), #495 (semi-global links, the
    expanded `std::globals`, `if let` bindings), #497 (`light_mode`).

## Context

#496 measured why 16,638 of 26,206 strict trees are still raw: their original's stores and
other ops that cannot fail write a `false` target of `error`, and the v2 backend writes
`false: true`. #498 gave the language a spelling for that (`dead_false = error`) and a
`TreePin` that carries it beside `node_order` and `share_tails`. The lifter did not use it
yet, nor the semi-global link (#495) or the light mode (#497).

As before, nothing changes what is claimed about a tree. Every tree is rebuilt and checked,
strict or `--behavioral`. A tree that a change breaks falls back.

## Decision 1: pins come from `match_tree_pin`

`pin_nodes` drops its own node-order matcher and calls `crate::node_order::match_tree_pin`,
which also finds `dead_false = error`. The header writes `TreePin::bhav_fields()`.
`share_large` builds its pin as a `TreePin` (`share_tails`, `dead_false_error: false`).

**`match_tree_pin_compiled` was evaluated and not adopted.** A pin is dropped when the
polished text no longer builds the pinned bytes. Over the corpus that happens 37 times on
10 trees (9 copies of one tree in one object family, plus one `share_large` pin). On each,
the exact pin equals the predicted one, so exactness recovers 0 trees.

**Result:** strict idiomatic trees go from 9,567 to 21,183 with this alone.

## Decision 2: the object's semi-global link is written

`Model::semi_global_link()` reads the original's `GLOB` name and keeps it when
`check_semi_global_name` accepts it. Then:

- the object header writes `semi_global: "Name";` after `attributes:`;
- the `extern mod semi_globals` the lifter declares for semi-global calls becomes
  `extern "Name" mod semi_globals`.

A library block is trusted to declare a tree's whole frame, so it proves a call to its
tree narrow (four argument words or fewer). The lift declares a semi-global tree with no
parameters when it only sees it called through the temps. Such a tree goes in the library
block only when its frame is known to be that narrow: a literal call of it, or a catalog
arity of 4 or fewer. Any other goes in an untagged `extern mod semi_globals_wide`, which
proves nothing (review round 2: a raw literal call added to such a tree could otherwise
have been taken as narrow under `dead_false = error`). That is 15 trees in 10 files, none
in the catalog.

Over the base install:

- **476 files had a `GLOB` the step 1 lift did not reproduce.** 231 are now byte-identical,
  244 are linked by name in the other byte form (length-prefixed instead of NUL-terminated;
  the engine reads both, and `object_gap` compares names), and 1 is left unlinked: its
  `GLOB` name is empty, which `semi_global:` cannot write.
- **27 files keep an untagged `extern mod semi_globals`.** They call semi-global trees but
  have no `GLOB` chunk.
- **About 2,000 trees per mode move from inconclusive to proven.** An unlinked object's
  semi-global calls could not be resolved; inconclusive goes from 20,684 to 18,661.
- **A caller's owner environment compares the `GLOB` by the name it links**, not by its
  bytes (review round 1). Otherwise a caller in an object linked in the other byte form
  stays inconclusive. It changes no lifted file or verdict in the base install (those
  callers' environments differ elsewhere too), but a test pins it.

## Decision 3: more raw blocks are written as builtins

`raw_builtin_body` (#496) gains:

| raw statement | written |
|---|---|
| `temp[k] := scope(24, n)` (n in 0..=9) | `temp[k] = stack_obj.neighbor.<name>;` or `stack_obj.neighbor.field[n]` |
| `temp[k] := scope(32, n)` (n in 0..=255) | `temp[k] = stack_obj.neighbor.<name>;` or `stack_obj.neighbor.person[n]` |
| `play_sound(N [, stack_obj \| true \| false]) pad(i: v, ..)` | `play_sound(N, stack_obj \| false, padI: v, ..);` |

- The neighbor names are the ones `std` already gives the cells (`neighbor_cell`). Only reads
  are rewritten.
- The sound must be a literal u16 with a pad, and no pad of bytes 0, 1 or 4 (the sound and
  its flags).

`if_let_spellings` gets its own budget, `POLISH_IF_LET_BUILDS`, raised from 8 builds (the
cell budget it borrowed) to 24. The new raw builtins expose more `if let` candidates per
file than 8 builds can decide.

## Decision 4: literal `if` chains are spelled `match`

`match_spellings` finds an `if X == v { } else if X == v { } ..` chain of 3 or more arms on
the same operand with distinct literal or `A::B` values (plus an optional `else`) and writes
`match X { v => { .. } .. _ => { .. } }`. It skips `raw { }` blocks and runs up to 3 passes,
outermost chain first. Each chain is one byte-gated group. A chain the gate refuses is not
proposed again, and the next pass looks inside its arms (review round 1). No lifted file
in the base install changes with it. The refusal is keyed by the chain's text, so a refused
outer chain whose inner chain is rewritten is proposed again, and an identical chain
elsewhere is skipped (review round 2). Both only leave an `if` chain unrewritten.

## Decision 5: late passes for what the polish makes

- **Engine fields again.** The polish writes new cell reads (`data[16]`, neighbor cells), so
  `polish` starts with a byte-gated `engine_field_fixes` pass. `data[16]` becomes
  `.light_mode`.
- **`lint_spellings` again after the polish.** A `play_sound` the polish writes can take the
  method form lint `prefer_method_syntax` asks for.
- **`loop_allows`.** A raw self-loop that becomes native code is now seen by lint
  `unbounded_loop_without_yield`; one that the polish made native leaves a stale allow.
  The pass lints the text once and moves the "loops on itself" allow to where it is needed.

## Decision 6: a layout anchor stays raw

A `raw { }` block that pins layout (`scope(..)` with a frame word, `pad`, `local[N]`) sets
`keep_layout` for its tree, which keeps the backend optimizer off. When the last such block
of a tree becomes builtins, the optimizer runs on the tree, and the bytes change whenever
it finds something to rewrite (it does not always: a tree whose only anchor is one padded
sound builds the same bytes either way).

That is the cause of the strict `manual_raw_has_builtin` findings: 120 candidate blocks,
118 pass alone, 95 are kept jointly, and each of the 25 left is the last anchor of its
tree, which fails the gate jointly with the others. The two inspected fail by a node-count
error and by differing bytes. A Track 8 attribute that keeps the layout without a raw
block would let them go.

## Remaining lint

| mode | rule | count | cause |
|---|---|---|---|
| strict | `manual_raw_has_builtin` | 25 | layout anchors (Decision 6) |
| strict | `frame_headroom` | 1 | a tree near the node limit whose `share_large` pin is dropped |
| behavioral | `prefer_if_let` | 6 | the `let..else` binding needs a frame word the pinned locals lack |
| behavioral | `manual_raw_has_builtin` | 2 | mixed blocks (also in step 1) |
| behavioral | `collapsible_if_chain` | 1 | its `match` spelling fails the byte gate |

`duplicate_std_global` goes from 8 to 0 in both modes, behavioral `untyped_engine_field`
from 48 to 0, and the 5 style rules stay at 0. No lint is suppressed.

## Results

The base install: 1,191 objects, 26,206 trees. v2.3 is `lift-v2-3-*`, step 1 is
`lift-v2-4-*` (#496), final is `lift-v2-4-final-*`.

**Behavioral:**

| | v2.3 | v2.4 step 1 | v2.4 final |
|---|---|---|---|
| idiomatic trees | 25,308 (96.6%) | 25,580 (97.6%) | 25,590 (97.6%) |
| identical / equivalent | 870 / 4,651 | 889 / 4,632 | 1,143 / 6,401 |
| raw-fallback trees | 897 | 625 | 615 |
| inline raw statements | 13,347 | 12,323 | 12,326 |
| inline raw lines | 76,439 | 57,515 | 57,502 |
| total lines | 936,369 | 912,746 | 912,940 |
| lint findings | 347 | 86 | 9 |

**Strict:**

| | v2.3 | v2.4 step 1 | v2.4 final |
|---|---|---|---|
| idiomatic trees | 9,523 (36.3%) | 9,567 (36.5%) | 21,244 (81.1%) |
| identical / equivalent | 1,113 / 4,408 | 1,116 / 4,405 | 4,234 / 3,310 |
| raw-fallback trees | 16,682 | 16,638 | 4,961 |
| inline raw statements | 3,441 | 3,517 | 25,087 |
| inline raw lines | 8,017 | 7,662 | 100,604 |
| total lines | 965,908 | 966,192 | 966,953 |
| lint findings | 14 | 12 | 26 |

Lint counts use the final lint binary on each sweep. Inconclusive is 18,661 in both final
modes.

**Feature counts, step 1 → final:**

| | strict | behavioral |
|---|---|---|
| `dead_false = error` pins | 0 → 11,800 | 0 → 10 |
| `node_order` / `share_tails` pins | 988 / 32 → 12,776 / 1,588 | 0 / 20 → 10 / 20 |
| `extern "Lib" mod` / untagged / `semi_globals_wide` | 0 / 420 / 0 → 393 / 27 / 10 | same |
| `match` | 12 → 205 | 1,585 → 1,625 |
| `if let Some` / `let..else` | 0 / 0 → 5 / 12 | 5 / 15 → 12 / 23 |
| `.light_mode` / `data[16]` | 0 / 53 → 43 / 10 | 0 / 48 → 48 / 0 |
| padded `play_sound` | 5 → 518 | |
| `.neighbor.` reads | 90 → 419 | |

The 10 strict `data[16]` left are inside labeled (raw-fallback) trees.

Invariants, both modes:

- 0 DIFFERENT;
- 0 errors or timeouts;
- 0 `fmt --check` dirty files;
- by `cmpbase` against `lift-v2-4-*`: no object has fewer idiomatic or proven trees.

**Strict inline raw rises from 3,517 to 25,087 statements.** 11,677 trees that were labeled
nodes are now v2 code, and they bring their raw blocks along (1,042 files). Labeled trees
are not counted as inline raw, so the rise is the price of the move, not a regression: a
raw-fallback tree is raw in full. Strict total lines rise by 761 (0.08%) for the same
reason.

## Consequences

- **Track 7 (lint):** the remaining findings in the table above.
- **Lifted files:** a call to a semi-global tree is spelled `semi_globals_wide::f()` when
  the tree's frame is not known to be narrow (Decision 2).
- **Track 8 (lowering):** an explicit keep-layout attribute for layout anchors (Decision 6),
  and a `let..else` that does not cost a frame word.
- **Follow-ups:**
  - the `collapsible_if_chain` whose `match` fails the byte gate;
  - a way to declare a semi-global tree's frame width for calls through the temps, so the
    15 trees in `semi_globals_wide` can join their library block;
  - the 27 files that call semi-global trees without a `GLOB` chunk;
  - an empty `GLOB` name, which `semi_global:` cannot write.
