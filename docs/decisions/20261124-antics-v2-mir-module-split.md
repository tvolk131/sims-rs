# `.antics` v2 Lowering: Module Split and MIR Layer

*Decided 2026-11-24.*

**Branch:** `t3/antics-v2-mir-refactor`  
**Track:** Track 8 (`.antics` v2, Wave A)

## Context

`v2/lower.rs` had grown to 16,470 lines in one file. It held:

- AST lowering;
- frame and slot bookkeeping;
- `stack_obj` save planning;
- specialization hooks;
- `.air` text emission.

`v2/opt.rs` (2,723 lines) held two things in one file:

- its own parsed form of a lowered body;
- the analyses and passes over that form, including the frame-allocation and entry-save passes.

PR H took nine review rounds, largely because a change to one concern meant reading all of them.

## Decision

The work is a refactor that is strictly behaviour-preserving. It adds no syntax and changes nothing in the parser or AST. It also makes no intended change to the output. The equivalence bar is **byte-identical** output.

### Changes

- **`v2/lower.rs` becomes `v2/lower/`**, with 24 child modules plus `mod.rs`.
  - The split commit moves every item verbatim. Its only edits are a `pub(super)` on moved items that had no visibility, and the module headers.
  - Later commits in this change edit the moved code only to route emission through `mir::Emitter`, `StackObjOp` and the typed writers (below), and to hand the end of `finish` to `mir::finish_body`. Each of those edits keeps the output byte-identical.
  - Types, structs and enums stay in `lower/mod.rs`, so their private fields stay visible to every child.
- **A new `v2/mir/` layer** takes what was mixed into `opt.rs`. See the module map below.
- **`opt.rs`** keeps only the optimizer passes over the MIR and their tests. It went from 2,723 lines to about 800.
- **`mir::Emitter`** replaces `TreeLowerer`'s `out` / `indent` fields. It writes lines at a depth and gives the marks that trial lowering uses: `mark`, `truncate`, `split_off`, `since`, `take_text` / `set_text` / `append`.
- **`mir::StackObjOp`** types the stack-object saves and plain stores the lowering generates (`Save`: `let X = stack_obj;`, `Set`: `stack_obj := X;`). `StackObjOp::text()` is the shared formatter, and `TreeLowerer::stack_op` is the helper that emits one.
  - The dedicated save and restore sites call `stack_op`.
  - The `__self_obj` entry save calls `stack_op`. The `__saved_stack_obj` entry save (prepended in `finish`) and the `for` cursor initializer format with `text()`.
  - The generic writers `emit_let` / `emit_store` route a stack-object operand to `stack_op`. They are used wherever a generated `let` initializer or `:=` destination can resolve to `stack_obj`. So a source `stack_obj = self.id;`, a `let saved = stack_obj;`, a `distance_to` snapshot, and the first (`:=`) store of `stack_obj = -x;` or `stack_obj = -1 + x;` are typed too. The `-=` / `+=` that follows such a first store is a compound write (see below).
  - The claim is about plain (`:=`) writes of `stack_obj` and `let X = stack_obj;` saves only. Stores whose destination is fixed and is never `stack_obj` are still formatted directly, including ones with a variable value. Examples: `temp[i] := 4;` from `job_data(4, 2, 5)`, `temp[0] := <career>;` from `job_data(career, 2, 5)`, the pinned `local[N] := obj;`, the dialog `temp` restores, and the generated counters and flags.
  - Out of scope: raw `.air` text, which bypasses `StackObjOp` (it keeps the lowering's existing whitespace normalization: lines trimmed, blank lines dropped, reindented), and compound writes (`stack_obj += x;`, `stack_obj -= x;`), which stay plain stores. The next PR makes the coverage total by construction, once the emitter builds typed instructions.
- **`mir::finish_body` with `FrameFacts`** is the per-tree backend pipeline. It takes over what `TreeLowerer::finish` ran inline:
  - the entry-save drop or the optimizer;
  - the pinned slot;
  - the raw constants;
  - `@slot(N)` placement.

### Module map

**`v2/lower/`**

| Module | Contents |
| --- | --- |
| `mod.rs` | All types, plus `lower_resolved_module` |
| `env.rs` | Dialog string table, enum/bitflags resolution, `ObjectEnv` |
| `object.rs` | `lower_object`, `resolve_variants` |
| `schema.rs` | State slots, BCON tables, tree signatures, object schema, guards |
| `trees.rs` | Standalone and guard trees, `.air` header, framed tree writer |
| `prim_table.rs` | `PRIM_BUILTINS`, `OP_BUILTINS`, builtin-name lookups |
| `tree.rs` | `TreeLowerer` core: `new`, `finish`, `line`, `fresh`, bindings |
| `stack.rs` | `stack_obj` tracking, saves and restores, `stack_op`, cursor checks |
| `scope.rs` | Scopes and cleanups: `defer`, locks, loop exits, cancel |
| `callable.rs` | Calls to user callables, value returns |
| `stmt.rs` | Statement dispatcher, range `for`, `let Some`, expression statements |
| `raw_consts.rs` | Raw-block constant ownership and collection |
| `raw.rs` | Raw `.air` parsing, validation, clobber facts |
| `match_lower.rs` | `match` lowering |
| `matching.rs` | Match arm and pattern helpers |
| `dialog.rs` | Dialogs |
| `builtins.rs` | Builtin arguments, primitive formatting, animation, `sleep` |
| `actions.rs` | Functionals, sounds, budget, find-best-object, create, slots, motives |
| `inline.rs` | Helper inlining |
| `lvalue.rs` | Places: l-value resolution, assignment, reads |
| `consteval.rs` | Compile-time evaluation and dead-branch checks |
| `expr.rs` | Value expressions |
| `cond.rs` | Conditions, `if`, short circuits, boolean returns |
| `ast_facts.rs` | Syntactic AST facts: writes, borrows, divergence, returns, `stack_obj` clobbering |
| `air_text.rs` | Small `.air` text helpers |

**`v2/mir/`**

| Module | Contents |
| --- | --- |
| `body.rs` | `Stmt` tree (`Inst` / `If` / `While`), block walkers |
| `text.rs` | `parse_body` / `print_nodes` |
| `inst.rs` | `InstKind` (the typed view of an instruction) and operand facts |
| `effects.rs` | `stack_obj` and last-result effects |
| `liveness.rs` | Local liveness |
| `frame.rs` | Pinned slot, `@slot(N)` placement, local renaming |
| `entry_save.rs` | Sim-tree entry save: forwarding and dropping it |
| `emit.rs` | `Emitter`, `StackObjOp` |
| `pipeline.rs` | `FrameFacts`, `finish_body` |
| `tests.rs` | Unit tests |

### Verification

**Differential harness.** It runs the base binary (b3758a7c) and the new one over the same corpus, and compares every observable output byte for byte. The corpus is:

- 1,341 `.antics` files:
  - every `.antics` in the repository;
  - every pilot and scratch object under `cache/scratch`, which includes the 13 Round 3 pilot objects;
  - 642 synthetic probes from earlier review rounds.
- 163 packs, including `open-pack`.

The outputs compared are:

- `expand`;
- `check`;
- `lint --json`;
- the sha256 of the `.iff` built from each file;
- the sha256 of every file each pack builds.

Two base runs agree with each other (0 differences). Every stage of this change also gives 0 differences.

### Staging

Turning the remaining text emission sites (about 300 `self.line` calls in `lower/`) into fully typed instructions, and moving the passes from text heuristics to typed effects, is left out of this PR. Doing it in one change would not keep the output byte-identical at a reviewable size. It is the next PR, done one family of instructions at a time.

That PR starts from the seams added here:

- **`Emitter`.** Every lowered line already goes through it, so it can start building `mir::Stmt` instead of text.
- **`InstKind`.** Today a view over the text; it can become a stored typed instruction.
- **`StackObjOp`.** Generated `let X = stack_obj;` saves and plain `:=` writes of the stack object are already typed (raw `.air` and compound writes excepted).
- **`FrameFacts`.** It carries the frame metadata.

The Round 3 fixes each become a local change:

- **Unread entry-save drop for dialog trees.** One condition in `mir/entry_save.rs`, using `FrameFacts::formats_text`.
- **Hidden `local: 0` slot accounting.** `FrameFacts::pinned` / `pinned_locals` in `mir/pipeline.rs`.
- **Inline-helper slot sharing.** `lower/inline.rs`, which no longer shares a file with the statement dispatcher.

The canonical `@context(inherit)` equivalence check is in the verifier. That is outside this change's files.

## What would reopen this

- A behaviour change found that traces to this refactor. The harness and tests say there is none.
- If moving items between modules makes rebases of in-flight v2 PRs costly enough that the split should have waited.
