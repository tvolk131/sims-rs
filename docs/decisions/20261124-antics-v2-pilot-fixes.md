# `.antics` v2: formatter, linter and `lift --v2` fixes from the 13-asset pilots

*Decided 2026-11-24.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/{fmt,lint,decompile}.rs`, the `lift --v2` path of `sims-antics`
- **Track:** Track 9
- **Follows:** `20261124-antics-v2-lint.md`, `20261123-antics-v2-fmt-and-decompiler.md`

## Context

Five pilot batches ported 13 objects by hand to idiomatic v2, proving every tree against the original. Their reports listed tooling friction in three areas:

- **Formatter:** `fmt` expanded `if c { return false; } // antics-lint: allow(x)` into a block and moved the comment after the closing `}`.
  - The directive then covered the wrong line, so the lint fired again after formatting.
  - Trailing `// gap:` notes moved the same way.
- **Linter:** too noisy to keep on.
  - `unused_decl` fired on locals that builtins write (`animate(local: x)`, `random_into(x, n)`), and on locals only a dialog's `$Local:N` token reads.
  - `stack_obj_clobber_after_self` fired after `stack_obj = saved;`.
  - `redundant_bool_return` suggested `return !!(...)`.
  - That rule and `negated_comparison` suggested rewrites that broke the proof, because they flip the tree's last result.
- **Lift:** a lifted object still needed manual work.
  - Hand renaming: `e0` BCON entries, `sound_N`, `global_N`.
  - Missing spellings for common shapes.
  - Catalog variants dropped.

Tracks 7 and 8 own the verifier and the v2 front end and lowering, so none of those files change here.

## Decision 1: end-of-line comments stay on the line they end

Before printing, the formatter moves a line comment that ends a one-line block back to the block's opening line. This applies to a comment on the same line as a `{ ... }` whose closing `}` is the last token before it. The comment becomes the first thing inside the block.

- `if c { return false; } // note` formats as `if c { // note` with the body on the next line.
- A second `fmt` pass is a no-op.
- `raw` blocks, and blocks whose inner tokens carry comments, are left alone.

The linter adds the other half: a directive trailing a line that opens a block (`{` last) covers the whole block, up to its `}`. A directive written either way therefore holds both before and after `fmt`.

## Decision 2: lint rules that follow what the compiled tree does

The linter expands each file once.

- It parses the lowered `.air` frames: which local slot holds which `let`, compiler temporary, or raw `local[N]`.
- It compiles from the same expansion for `frame_headroom`.

Rules that depend on the frame use the real layout, not a guess from source order.

| Change | Rule |
|---|---|
| "read before anything assigns it" moved to its own code | **`uninitialized_local_read`** (new) |
| Builtins that write a local assign it: `random_into`'s first positional argument, and `local:` / `icon:` of `animate` / `set_headline` / `push_interaction` (the lowering's own list) | `uninitialized_local_read` |
| A `let` whose slot a `$Local:N` / `$TimeLocal:N` token reads is used. When it isn't, the note names the slots the tokens read and the slot the `let` is on | `unused_decl` |
| A `const` whose value a `raw` block uses as a literal index (`attr[7]`; not `local` / `temp` / `param`) is used, because `raw` does not resolve `const` names | `unused_decl` |
| A token's slot holds no local, a compiler temporary, or an inlined helper's local. The note lists the tree's slot layout, and states that the verifier does not compare string tokens | **`dialog_local_slot_mismatch`** (new) |
| A `raw` block that v2 code follows and that ends in a bare `call N(...);`: its false edge exits the tree. Help: `if call N(...) {}` | **`raw_bare_call_false_exit`** (new) |
| A `(sim: Sim)` tree with no parameters whose `raw` reads `param[n]` past its `@frame(args:)`. `sim` is not an argument word, and `raw` is not checked against the frame | **`missing_sim_frame_args`** (new) |
| `stack_obj = saved;` restores `self` when `saved` took the handle before any swap. Help now offers `stack_obj.<field>` for writes meant for the other object | `stack_obj_clobber_after_self` |
| Not reported when the condition contains a comparison: `return a op b;` lowers to the flipped compare and ends on the opposite last result. `!c` returning false suggests `return c;`, not `!!` | `redundant_bool_return` |
| Reported only for an `if` / `while` testing `!(a op b)` whose both successors run another node before the tree can end. Otherwise the compare's own result can be the tree's last result | `negated_comparison` |
| The message names the culprit: the raw slots below the named locals, and repeated `let`s or compiler temporaries | `frame_headroom` |

### Why heuristics, and which ones

- **`raw_bare_call_false_exit`** fires only when v2 code follows the block. Inside a block of `.air`, the false exit is the author's own `.air`. A lifted tree mirrors the original's.
- **`negated_comparison`** treats `let` and stores to plain locals as not running a node, because the optimizer may drop them.
- **`dialog_local_slot_mismatch`** checks only what the frame shows. It cannot know which `let` the author meant when the slot holds a different `let` of the same tree.

## Decision 3: `lift --v2` names and spellings (Sol, reviewed here)

Every new spelling is an extra idiom: a tree that does not prove falls back to the narrower form.

- **Names:**
  - BCON entries from the matching `STR#` and `TRCN` labels.
  - Sounds from their FWAV event names.
  - Global and semi-global calls from the library BHAV labels, with `--globals` / `--semiglobal` / `--install`.
  - Each name falls back to `eN`, `sound_N` or `global_N`.
  - Catalog GUID naming is not done.
- **Spellings:**
  - `dialog(...)?`;
  - padded `action_label(...)`;
  - `stack_obj.sprite_flags[i]`;
  - `destroy_self(no_yield: true)`;
  - `animate(..., local: slot_N)`, with the event local's slots declared, and flag `0x20` written as `local:`;
  - known-arity literal calls (`f(1, 2)`) to private trees and libraries;
  - no dead local byte on a flags-0 animate.
- **Checker:**
  - The lift's own comparison drops Animate operand byte 2 when flag `0x20` is clear, because the VM reads it only then (`sims-vm` `animate.rs` `store_event`).
  - Every OBJD's bytes are compared, not only the variant words.
- **Variants:**
  - Catalog variants are kept when the primary OBJD is not a multi-tile master.
  - All 68 base-game multi-OBJD files are tile groups. v2's `variant` cannot spell those, so they stay reported as not reproduced.

## Results (local only, aggregates)

- **Base game, 2722 trees:**
  - 0 different, 0 objects regressed;
  - idiomatic 848 → 850;
  - raw statements 1253 → 1226;
  - the same with `--install`.
- **Open pack, 116 trees:** 116 proven, 112 idiomatic, unchanged.
- **Warnings per corpus, before → after:**

  | Corpus | Before | After |
  |---|---|---|
  | open-pack | 2 | 2 |
  | fridges pilot | 21 | 8 |
  | open-pack lifted | 38 | 2 |
  | base-game lifted | 125 | 48 |

  - The base-game figure is for the new lift output, which spells more conditions as calls.
  - Its 6 `raw_bare_call_false_exit` hits are real false exits followed by `return true;`.
- Every committed fixture is synthetic, with GUIDs in `0x7E00_xxxx`.
