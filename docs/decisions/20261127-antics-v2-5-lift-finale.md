# `lift --v2` v2.5 step 2B: `keep_layout` for layout anchors, semi-global frame widths

*Decided 2026-11-27.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/v2/decompile.rs`
  - new `tests/integration/v2_5_lift_finale.rs`
  - `tests/integration/v2_4_lift_finale.rs` (the semi-global width test)
- **Track:** Track 9 (`lift --v2`), `.antics` v2.5 step 2B
- **Builds on:**
  - [`20261126-antics-v2-5-compiler.md`](20261126-antics-v2-5-compiler.md) (#503, step 1A:
    `#[bhav(.., keep_layout)]`, `#[frame(args = N | unknown)]` on extern fns);
  - [`20261126-antics-v2-5-lift-locals-and-census.md`](20261126-antics-v2-5-lift-locals-and-census.md)
    (#505, step 1B: the baseline `lift-v2-5-step1-*`);
  - #507 (step 2A, Track 7): `manual_raw_has_builtin` advises `keep_layout` for a layout
    anchor (it still reports any raw builtin block a pinned tree keeps), and
    `prefer_if_let` knows fixed frames.

As before, nothing changes what is claimed about a tree. Every tree is rebuilt and checked,
strict or `--behavioral`, and a tree that a change breaks falls back.

## Context

#505 left 26 strict and 3 behavioral lint findings over the corpus. 25 strict and 2
behavioral were `manual_raw_has_builtin`: a `raw { }` block that holds only a builtin
statement. The polish could not write those blocks as the builtin. Each was the last raw
block of its tree. A raw block keeps the backend pass off the whole tree, so writing the
last one natively let the optimizer rewrite the tree, for example by merging two arms'
equal `refresh; return true` tails. The byte gate refused it. #503 added
`#[bhav(.., keep_layout)]`, which keeps the layout as such a block did.

(The v2.4 ADR called the 2 behavioral blocks mixed blocks. They are not. Both are a single
`scope(27, 0) := 0;` store in a loop, which is `stack_obj.sprite_flags[temp[0]] = 0;`, and
they are anchors like the strict ones.)

The v2.4 lift declared a semi-global tree that is only called through the temps in an
untagged `extern mod semi_globals_wide` block, unless its catalog arity was 4 or less. A
library block (`extern "Lib" mod semi_globals`) is trusted to declare a tree's whole
frame, so a parameterless entry there would have claimed a frame of 0. That covered 15
trees in 10 files. #503 lets an extern fn declare its width.

## Decision 1: the last layout anchor goes with `keep_layout`

The raw-builtin polish step now has two stages:

1. As before, a joint byte-gated search keeps the blocks that build the same bytes when
   written as their builtins (`keep_joint`, `POLISH_RAW_BUILDS`).
2. Each block refused in stage 1 is paired with `, keep_layout` inserted before the `)]` of
   its tree's `#[bhav(..)]` (`keep_layout_pin`). The pairs are searched jointly on top of the
   blocks already kept, with their own budget (`POLISH_LAYOUT_BUILDS` = 8).

Each pair is tried as a unit, so a pin never lands without the block it replaces. Every
candidate is checked against the whole object's bytes, the same gate as stage 1.

`keep_layout_pin` walks back to the block's tree header. It gives up in these cases:

- The tree's header is `#[id(..)]`: a lifecycle tree with no frame header. A framed
  `init` or `main` is written with `#[bhav(..)]` and takes the pin like any tree.
- The header already has `keep_layout` outside its string literals. A label that holds the
  word is not a pin (`unquoted`).
- The header does not end its line with `)]`.

Two blocks in one tree share one pin. `unique_rewrites` drops the duplicate insert, so the
joint text has it once.

This is fail-closed. A pin is tried only with a block that stage 1 refused, whatever else
the tree keeps raw. Not every raw block keeps the layout (a raw `temp[1] := 1;` does not), so
a tree that keeps one may still need the pin. A pin that changes the bytes is refused like
any other spelling. When the budget runs out, the blocks not yet tried stay raw.

## Decision 2: semi-global trees declare their width in the library block

Every semi-global tree (id 8192 and up) of a linked object is declared in its library block.
`semi_global_width` gives the frame annotation:

- **None** when the tree is known to be narrow: it has a literal call, or its catalog arity
  is 4 or less. Positional parameters, or none, describe it as before.
- **`#[frame(args = N)]`** when the catalog gives an arity N in 5..=255.
- **`#[frame(args = unknown)]`** when the tree is not in the catalog, or its catalog arity
  does not fit in a byte.

An unlinked object has no library block, so it is unchanged: its semi-global calls stay
where they were.

Every named call to a semi-global tree is `semi_globals::` (a call the lift keeps raw,
`call N(temps)`, is unchanged). `semi_globals_wide` is gone, along with
`SEMI_GLOBALS_WIDE` and `semi_global_module`.

`args = N` takes a byte. A catalog arity above 255 (no install gives one, but a caller of
the library API may) is declared `unknown`, which proves no more.

Over the corpus, all 15 trees (ids 32767 ×8 and 65535 ×7, neither in the catalog) are
`#[frame(args = unknown)]`, as #503's hand rewrite of the 10 files predicted. No tree is
declared `args = N`, but the synthetic test covers it (8202, catalog arity 6). The width
proves nothing: no literal call to an `unknown` tree is taken as narrow. The lifted files
build the same bytes, and every tree keeps its check.

## Decision 3: no new `if let` work

The coordinator's brief counted 68 behavioral and 36 strict `prefer_if_let` sites. Track 7
counted them over the v2.4-final corpora. The step-1 lift (on #503's fixed-frame option
bindings) already writes them as `if let` / `let..else`. #507's lint finds 0 `prefer_if_let`
in `lift-v2-5-step1-*`, so this step changes nothing for them.

| mode | `if let` / `let..else`, v2.4-final | v2.5 step 1 and final |
|---|---|---|
| strict | 5 / 12 | 8 / 45 |
| behavioral | 12 / 23 | 15 / 88 |

## Results

The base install has 1,191 objects and 26,206 trees. Final sweeps: `lift-v2-5-final-*`,
built on `origin/main` with #507.

| | strict v2.4-final | strict v2.5 step 1 | strict v2.5 final | behavioral v2.4-final | behavioral v2.5 step 1 | behavioral v2.5 final |
|---|---|---|---|---|---|---|
| idiomatic trees | 21,244 (81.1%) | 21,244 | 21,244 | 25,590 (97.6%) | 25,590 | 25,590 |
| identical / equivalent | 4,234 / 3,310 | 4,234 / 3,310 | 4,234 / 3,310 | 1,143 / 6,401 | 1,143 / 6,401 | 1,143 / 6,401 |
| raw-fallback trees | 4,961 | 4,961 | 4,961 | 615 | 615 | 615 |
| inline raw statements | 25,087 | 24,509 | 24,484 (−25) | 12,326 | 11,896 | 11,894 (−2) |
| inline raw lines | 100,604 | 98,820 | 98,795 | 57,502 | 56,118 | 56,116 |
| inline raw blocks | 14,962 | 14,927 | 14,902 | 9,409 | 9,318 | 9,316 |
| total lines | 966,953 | 966,720 | 966,640 | 912,940 | 912,571 | 912,537 |
| `semi_globals_wide` blocks | 10 | 10 | 0 | 10 | 10 | 0 |
| `keep_layout` trees | 0 | 0 | 25 | 0 | 0 | 2 |
| lint findings | 26 | 26 | 1 | 9 | 3 | 1 |

These invariants hold in both modes:

- 0 DIFFERENT.
- 0 errors or timeouts.
- 0 files dirty under `fmt --check`.
- `cmpbase` against `lift-v2-5-step1-*` finds no object with fewer idiomatic or proven trees,
  and no file with more raw statements.

The only files that change are the anchor trees' files and the 10 `semi_globals_wide`
files: 32 strict and 12 behavioral. The only lines that change are:

- the pinned header;
- the builtin written in place of its raw block;
- the moved declarations;
- `semi_globals_wide::` becoming `semi_globals::`.

**Remaining lint** (no lint is suppressed):

| mode | rule | count | cause |
|---|---|---|---|
| strict | `frame_headroom` | 1 | unchanged since #499 |
| behavioral | `collapsible_if_chain` | 1 | its `match` spelling fails the byte gate (unchanged) |

## Consequences

- **Track 8 (lowering), unchanged from #505:**
  - a writable `local[N]` that keeps the layout. The 3,722 strict and 2,762 behavioral raw
    local writes wait for it;
  - a way to keep an original's unreachable nodes (3,836 of the 4,961 strict raw-fallback
    trees);
  - a spelling that keeps identical `if` arms unmerged.
- **Track 7 (lint):** nothing new. The 2 findings left are known, and neither is a lift
  spelling the lint could suggest.
- **Track 9:** the v2.5 lift is done. What remains of the strict raw gap is Track 8's work
  above.
