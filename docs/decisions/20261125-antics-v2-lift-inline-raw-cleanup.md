# `lift --v2`: fewer inline `raw {}` statements

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/decompile.rs`
- **Track:** Track 9 (`lift --v2`)
- **Builds on:**
  - [`20261125-antics-v2-lift-behavioral-mode.md`](20261125-antics-v2-lift-behavioral-mode.md)
    (the behavioral sweep and its follow-ups);
  - [`20261125-antics-v2-pr-i2-pilot-fixes.md`](20261125-antics-v2-pr-i2-pilot-fixes.md) (a
    local `extern mod globals` merges with `use std::globals::*;`; `me` in inherit trees).

## Context

After `--behavioral`, 96.5% of trees lift idiomatically, but tens of thousands of statements
inside them still sit in inline `raw {}` blocks: moves between unnamed locals, the stack
object's and this object's cells, padded `expr(..)` primitives, bit operations, and
conditions with no spelling that take a whole `if` with them. Raw lines also read constants
as `bcon[N]`. Every library tree a file declares locally sat in a `more_globals` block, which
#461 made unnecessary.

None of this changes what is claimed about a tree. Every new spelling is checked like any
other: the lift recompiles the file and proves each tree (strict or `--behavioral`), and a
tree that a spelling breaks falls back as before. The full #444 soundness contract is
unchanged.

## Decision 1: one local `extern mod globals`

Since #461 merges a local `extern mod globals` with `use std::globals::*;`, library trees the
std module does not declare go in a local `globals` block. `more_globals` stays only where the
merge would be refused: a local declaration of a tree id or a name that std also declares
(with another arity or signature).

## Decision 2: new spellings

Each is an extra idiom (`Translator::extra`), so a tree it breaks is rewritten without it:

- **Named locals.** Every local a tree reads or writes is declared `#[slot(i)] let slot_i:
  i16;`, as the locals builtins address already were. If a round with these names is not
  proven or does not compile, the tree first goes back to naming only the builtins' locals (a
  pinned word can take the frame word an idiom such as `distance_to(self.id, ..)` needs), and
  only then drops its other extras.
- **Cells.** This object's attributes in an object tree (`self.attr[i]`), the stack object's
  attributes, data, motives and person data (`stack_obj.*`, or `self.*` in a Sim tree that
  never moves `stack_obj`), and a Sim tree's own slots (`sim.slot[i]`).
- **Ids.** `stack_obj := <handle>.data[11]` is `stack_obj = <handle>.id`.
- **Padded primitives.** `expr(..) pad 0xNN` is the plain assignment or comparison, and a
  padded primitive takes its unpadded spelling. The checker canonicalizes bytes the VM never
  reads, so the pad is not observed. `!` now parenthesizes any binary condition, not only a
  compare node.
- **Bits.** `flag_test`, `flag_set` and `flag_clear` on any `i16` cell, not only on `.flags`
  places. v2 passes the bit number through for `i16` operands.
- **Inherit trees.** A sound with the frame object as target is `play_sound(s, me)`. The bare
  form depends on the caller's context, which `context = inherit` refuses. Callees of an
  inherit tree are written inherit too: lowered in both contexts, it may call only inherit
  trees. A callee whose body depends on the context fails as inherit and drops back to its own
  context. That is tried before its other extras, which may still hold there.
- **Self-loops.** A loop whose whole body is one primitive (the original's node loops on
  itself) carries the same `unbounded_loop_without_yield` allow that breakpoint traps already
  had.
- **A search moves `stack_obj`.** A Sim tree that calls `find_best_interaction()`, steps
  `set_to_next` into `stack_obj` (its default destination), draws a random number into
  `stack_obj`, counts it with `inc_lt`/`dec_gt`, or assigns it through `expr(..)` names the
  stack object's cells
  `stack_obj.*`, so `self` never means the found object (lint
  `stack_obj_clobber_after_self`).

## Decision 3: BCON paths in raw lines

Raw lines (inline blocks and whole raw trees) write `bcon[addr]` of the object's own tables
as `Table.entry`, the path idiomatic code already uses. A raw block resolves the path to the
same address, so the nodes do not change. Library tables and labels `.air` reserves keep
`bcon[addr]`.

- In an idiomatic tree's inline blocks the names are an extra like the others.
- In a whole raw tree they have their own switch. A round that does not compile, blamed on
  the tree or on no tree, turns them off first. An unproven raw tree loses them only once it
  has no lower style left. Its failure is about structure, which the names do not change,
  so dropping them earlier would cost a round and the names.

## What stays raw, and why

- **Temps calls to a tree declared with literal parameters.** v2 passes non-constant
  arguments through `let` copies, which need frame locals the pinned header does not have.
  Such calls stay `call N(temps)`.
- **Calls across contexts** where the callee could not be written inherit (its body depends
  on the context, or its inherit form needs more locals than the header gives).
- **Conditions on primitives v2 wraps in a cursor save** (`create_object`, push-interaction,
  dialogs, relationships): the save needs a frame local.
- **`scope(N, i)` operands without a v2 field** (scopes 21, 27, 30, ..): Track 8 is adding
  fields for some.

## Results (full corpus, same install)

Before is `main` before this change; after is this change. Both are measured on 1,192
candidate entries (1,191 objects lifted, 0 timeouts) in each mode.

| Metric | Strict before | Strict after | `--behavioral` before | `--behavioral` after |
| --- | --- | --- | --- | --- |
| Identical / equivalent / inconclusive / omitted | 886 / 4,635 / 20,684 / 1 | 884 / 4,637 / 20,684 / 1 | 878 / 4,643 / 20,684 / 1 | 870 / 4,651 / 20,684 / 1 |
| DIFFERENT | 0 | 0 | 0 | 0 |
| Idiomatic / raw-fallback trees | 8,472 / 17,733 | 8,482 / 17,723 | 25,296 / 909 | 25,296 / 909 |
| Inline `raw {}` statements | 7,580 | 3,694 | 78,794 | 29,997 |
| `raw {}` blocks (inline and whole-tree) | 22,597 | 19,943 | 49,001 | 23,924 |
| Lines inside `raw {}` blocks | 688,611 | 678,466 | 480,974 | 268,069 |
| `bcon[N]` references | 36,235 | 4,602 | 25,785 | 4,870 |
| Files with `more_globals` | 862 | 105 | 862 | 105 |
| Lines | 978,887 | 971,964 | 1,200,670 | 1,140,471 |
| fmt-dirty files | 0 | 0 | 0 | 0 |
| Lint findings | 7 | 12 | 102 | 102 |

- **Verdicts.** Every inconclusive tree is still inconclusive for an object-level reason, as
  before. Two strict trees and eight behavioral trees move from identical to equivalent: they
  are now written idiomatically, so their nodes are no longer byte-identical.
- **Lint.**
  - Strict: 5 more `negated_comparison` findings, from the 10 trees that now lift
    idiomatically. They come from the original's negated compare nodes, as in behavioral mode.
  - Behavioral: the total is unchanged. One `frame_headroom` moves to one
    `collapsible_if_chain`.
- **Library tables.** The remaining `bcon[N]` references are mostly library tables and
  labels `.air` reserves.
- **Time.** Within noise (the host is shared). Summed per-file lift time is 679 s → 717 s
  strict and 732 s → 938 s behavioral. Run alone, the slowest file took 14 s.

## Follow-ups

- **Lint: BCON entries read by name inside raw code.** `unused_decl` counts a raw block's
  `bcon[N]` as a read of every entry, but not a named path inside raw code. No corpus file
  hits this, since each still has a numbered read or an idiomatic one. A file whose raw blocks
  read an entry only by name would get a false `unused_decl`. (Track 7, `lint.rs`.)
- **Header locals for v2 helpers.** Several remaining raw shapes need one more frame local
  than the original header pins. Behavioral mode does not compare frame storage, so it could
  allow a larger `locals` count there.
