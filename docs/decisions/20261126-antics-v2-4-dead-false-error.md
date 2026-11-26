# `.antics` v2.4 Step 2: `dead_false = error`, and staging kept off beside frame-word text

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/{ast,parser,lower,node_order}.rs`
  - `src/v2/{ast,attrs}.rs`, `src/v2/lower/{trees,callable}.rs`
  - new `tests/integration/v2_4_dead_false.rs`
  - no edits to `src/v2/lint.rs`, `src/v2/decompile.rs` or `open-pack/`
- **Track:** Track 8 (compiler), `.antics` v2.4 Step 2
- **Builds on:**
  - [`20261126-antics-v2-4-lift-scope27-and-joins.md`](20261126-antics-v2-4-lift-scope27-and-joins.md)
    (#496, Decision 4: the strict-gap census);
  - [`20261126-antics-v2-3-step2-node-order-and-shared-tails.md`](20261126-antics-v2-3-step2-node-order-and-shared-tails.md)
    (#491: `share_tails`, `node_order`);
  - [`20261126-antics-v2-4-light-mode-glob-option-alias-self.md`](20261126-antics-v2-4-light-mode-glob-option-alias-self.md)
    (#495: the text guard on option bindings).

## Context

Some statements cannot fail: their instruction always branches true
(`verify::is_always_true_instruction`). Examples are an Expression assignment, Sleep and Play
Sound. Such a node's false branch is dead, but it still holds a byte:

- the original compiler writes `false: error` (0xFD);
- the v2 compiler writes `false: next`, the same node as the true branch.

Track 9's census (#496) found that this byte alone keeps most strict-mode trees from pinning to
the original's nodes.

## Decision 1: the original writes `false: error` almost everywhere

A census probe was run once over the original corpus and is not committed. It covered all 1,192
object files: 26,206 trees, and the 168,816 nodes among them whose instruction always branches
true. Their false branches go:

| false branch | nodes |
|---|---:|
| `error` (0xFD) | 168,598 (99.87%) |
| the true branch's node (`next`) | 185 |
| another node | 15 |
| another return | 18 |

Of the `next` nodes, 182 are Find Location in mode 1 (take the object off the lot). The authors
wired that statement as if it could fail.

| trees | count |
|---|---:|
| no node that always branches true | 4,409 |
| every such node `false: error` | 21,601 |
| `error` and `next` mixed | 166 |
| `error` and another node or return mixed | 30 |

A tree-level flag therefore covers 21,601 of the 21,797 trees with such a node (99.1%). A
per-node mask would add at most 196 trees, and every statement would carry a marker. It is not
worth the clutter.

## Decision 2: `#[bhav(.., dead_false = error)]`

- **Syntax.** v2 takes `dead_false = error` in `#[bhav(..)]` (and `#[frame(..)]`). The `.air` tree
  header takes `dead_false: error`. `error` is the only value. Without the field, a dead false
  branch goes to the next node, as before. A duplicate field, another value or a bare
  `dead_false` is an error.
- **Effect.** The field changes one thing: a statement node the compiler emits (`emit_stmt` in
  `lower.rs`) gets `false: error` when its instruction always branches true. This covers:
  - assignments, local copies and call staging stores;
  - Idle's parameter stores and Sleep;
  - a primitive written as a statement, and an inline raw op.
- **Untouched.** Conditions, calls, and branches written out (`raw { @n0: .. -> true: @n1,
  false: @n1; }`) keep their branches.
- **Order with the other rewrites.** The flag applies when nodes are assembled, so `share_tails`
  and `node_order` run after it, as they would on the original's bytes.
- **Wide inline calls.** A callee whose frame holds more than four arguments reads the rest from
  the bytes of the nodes after an inline call, false branches included. The flag changes those
  bytes, so it takes the same fail-closed checks as `share_tails` and `node_order`:
  - v2 refuses such a call (`fn 't' has 'dead_false', which rewrites false branches, ..`);
  - `.air` needs each callee outside the module listed in `narrow_callees`, and checks a sibling
    tree's frame.
- **Soundness.** The predicate is the verifier's own. Each opcode it lists cites its VM source.
  For the two most common, Expression assignments either branch true or trap, and Sleep either
  branches true or yields. The false branch is never taken, so the tree behaves the same either
  way: only its bytes change.
- **Byte identity.** The field is opt-in. Every existing source, `open-pack` included, builds
  byte-identical to main.

## Decision 3: `node_order::match_tree_pin` finds the whole pin

`node_order.rs` gains the helpers a decompiler needs to write the flag:

- `wants_dead_false_error(original)`: the tree has a node that always branches true, and every
  such node's false branch is `error`.
- `with_dead_false_error(compiled)`: a tree built without the flag, as the flag builds it. Every
  node that always branches true and whose false branch equals its true branch gets `false:
  error`.
  - The one difference from the compiler is a raw node written out with both branches to one
    place. The compiler leaves that node alone.
  - An original that wants the flag has no such node, so its pins come out the same.
- `TreePin { dead_false_error, share_tails, order }` with `bhav_fields()`, and
  `match_tree_pin(compiled, original)`:
  - It returns `None` when the nodes already match.
  - It tries the plain tree, then the flag, whenever the original writes any dead false branch
    `error`.
  - The flag is tried two ways:
    - every such node flipped;
    - the nodes whose instruction the original leaves with a live false branch kept, as a source
      writing them out raw gets them.
  - For each candidate, it tries the nodes as they stand (a node no branch reaches matches only
    this way), then a `node_order`, then `share_tails`, as it stands or with a `node_order`.
  - It returns the first pin that makes the nodes identical, or `None`.
- **The pin is a prediction.** Bytes cannot tell a statement from a raw node written with both
  branches to one place. A caller compiles the tree with the pin and keeps it only when the nodes
  come out identical, as the lifter already does for every pin.
- `match_tree_pin_compiled(compiled, flagged, original)` makes no prediction. It takes the
  source compiled once more with `dead_false = error` and tries that tree in place of the
  rewrites. It finds the pin even when a statement and a raw node hold the same instruction.

## Decision 4: what it unlocks (measured)

**Method.**

- The corpus sweep (`cache/track9/corpus-tools/corpus_sweep.py`) ran in strict mode, twice:
  - main at `88e798d3b`;
  - this branch, with a local probe build in which the lifter's `NodePin::matching` calls
    `match_tree_pin` and writes `dead_false = error`.
- The probe edit to `decompile.rs` is not committed: that file is Track 9's.
- The numbers are from after both review rounds' fixes. Round 1's added 65 idiomatic trees,
  from the broader flag candidates and the direct match (Decision 3); round 2's changed none.

| | main | with `dead_false` pins |
|---|---:|---:|
| idiomatic trees | 9,567 (36.5%) | **21,183 (80.8%)** |
| raw-fallback trees | 16,638 | **5,022** |
| Identical | 1,116 | 3,096 |
| Equivalent | 4,405 | 2,425 |
| DIFFERENT / errors | 0 / 0 | 0 / 0 |
| trees written with `dead_false = error` | 0 | 11,740 |
| `fmt --check` dirty files | 0 | 0 |

**Result.** 11,616 trees that strict mode left raw now compile to the original's nodes exactly,
so they are written idiomatically. This is most of #496's 9,669 + 3,949 trees.

**Lint.** Findings on the lifted corpus go from 4 to 102: 89 `manual_raw_has_builtin`, 7
`collapsible_if_chain`, 3 `prefer_if_let`, 2 `untyped_engine_field` and 1 `frame_headroom`. They
are on the newly idiomatic trees' inline raw blocks, which is the lifter's to tidy.

## Decision 5: in-place call staging is kept off beside frame-word text

#495 kept option bindings in their temps out of an object whose text may name frame words:

- a tree that formats `$Local:N` reads its own frame;
- that frame holds whatever an earlier frame left there.

So any tree's frame words are observable there. In-place call staging (#480) leaves fewer frame
words than the copying form, but it checked only the running tree's own text.

`copy_staging()` now includes the object-wide text check, which covers both passes. The check
holds only when some tree of the object formats text: a string that no tree formats reads no
frame word. A finished tree that formats text sets `saw_text_tree`, and the object is lowered
again with the fact, as for global 34.

- a tree with no text of its own stages a call in the copying form when a sibling tree shows
  `$Local:0`;
- the regression test fails without the change.

**Compatibility.** The copying form needs frame words for its copies. So a tree that pins its
frame too small for them (`#[bhav(.., locals = 0)]` calling `g::two(temp[1], temp[2])`) beside
such a sibling is now refused: "declares 0 locals but requires at least 2". This is the existing
rule for every copy-staging object (global 34 reads, temp loops, a fixed entry). Neither
`open-pack` nor any lifted corpus source is affected: both build unchanged. The fix is to drop
the `locals` pin or raise it.

## Review

Codex GPT-6.2 Sol, round 1, found five issues, all fixed:

- (high) the flag rewrote bytes that a wide inline call reads: it now takes the wide-call checks
  (Decision 2);
- (medium) the pin helper predicted the flag for a raw node the compiler keeps, and missed valid
  pins: a mixed original, and a node no branch reaches. Decision 3 now covers both, and states
  that the pin is a prediction;
- (medium) a `locals = 0` source beside frame-word text is refused (Decision 5, compatibility);
- (low) `.air` took a duplicate `dead_false`: it is refused.

Round 2 found four more, all fixed:

- (medium) an object's string naming `$Local:0` turned on the staging guard even though no tree
  formats text, so an unflagged source changed bytes. The guard now needs a tree that formats
  text (Decision 5);
- (medium) the pin helper could not find the flag when a statement and a raw node hold the same
  instruction: `match_tree_pin_compiled` takes the flagged build;
- (low) after `share_tails` the helper tried only a renumbering: it now checks for equality as
  well;
- (low) a narrow callee called from a raw block as `subroutine(N, ..)` or `subroutine N(..)` was
  not listed in `narrow_callees`: the raw-block scanner now reads all three spellings.

## Consequences

- **Track 9 (lift):** switch `NodePin::matching` to `node_order::match_tree_pin` (or to
  `match_tree_pin_compiled`, with a second build of the source) and write
  `TreePin::bhav_fields()`. That is the whole probe. The lifter's `share_large` pins keep
  `dead_false_error: false`. Tidy the inline raw blocks that newly idiomatic trees carry (the
  lint findings above). Rebaseline the strict corpus.
- **Track 7 (lint):** `dead_false` is a new `#[bhav(..)]` field. Lint needs no rule for it, but
  any table of known header fields should list it.
- **Not done:**
  - a per-node marker for the 196 mixed trees;
  - a way for a hand-written tree to keep one dead false branch `next` under the flag.
