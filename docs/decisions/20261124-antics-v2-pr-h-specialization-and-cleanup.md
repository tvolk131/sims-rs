# `.antics` v2 PR H: Branch Specialization, Pilot Regressions, and Raw-Eliminating Features

*Decided 2026-11-24.*

**Branch:** `antics/v2-pr-h-specialization-and-cleanup`  
**Track:** Track 8 (`.antics` v2, PR H)

## Context

The R2 round of the multi-asset pilots rebuilt four batches of commercial objects on PR G (`a9eacd99`). The runs were local, and nothing from them is in the repository. Their reports raise three things:

- **Regressions in PR G.**
  - The entry save survives out-operand writes and raw blocks.
  - `$Local` text loses its locals when they are coalesced.
  - `relationship(value:)` accepts only a local.
  - `return a != b` changed polarity.
- **A missing idiom.** One runtime test picks compile-time operands, and the dependent nodes are copied under each test. Without it, sources need a local, or a raw block per arm.
- **Remaining raw sites.** These come from:
  - helper calls that cost locals;
  - stack-cursor loops;
  - `create_object` into the stack object;
  - pinned frame words;
  - `const` names inside `raw { }`;
  - lifecycle trees with Maxis headers;
  - calls across tree contexts.

Constraints:

- **Lowering targets.** Everything lowers onto existing `.air`.
- **Files changed.**
  - v2 changes stay in `src/v2/{ast,parser,lower,opt,specialize,mod}.rs`.
  - The `.air` compiler (`src/lower.rs`) gains one read-only query: `air_local_usage`.
  - The verifier, `fmt`, `lint` and `decompile` are untouched; parallel tracks own them.
- **Synthetic tests.** Every test is synthetic, in `tests/v2_pr_h*.rs`. `open-pack/` builds unchanged.

## Decisions

### 1. Tier 1: PR G regressions

- **H-1: Out-operands are writes.**
  - The entry-save flow analysis (`opt::drop_unread_entry_save`) counts each out-operand as a write on the exits where the VM writes it:
    - `random_into` writes on both exits;
    - `animate(local:)` writes on the false exit (an event);
    - a relationship read writes on the true exit.
  - A `break` carries its state to the loop exit, and the states meet there. `return` and `continue` are vacuous.
  - Raw code is split one statement per line before the analysis.
  - Only positional branch targets block the drop. A raw `local` declaration still keeps the save.
- **H-2: `sim.attr` and `sim.data` never clobber the stack object.** A Sim tree that only touches Sim cells drops its save.
- **H-3: `$Local` text keeps its locals.**
  - In a tree that formats text (`dialog`, `action_string`), every source local keeps its store and its own word: `let`, inlined-helper `let` and `@slot` alike.
  - A string can name a word by number, so coalescing would change what it reads.
- **H-4: `relationship(value:)` takes cells and constants.**
  - With flag 4 the VM reads `value:`, so any i16 value works, computed into a temporary when it needs nodes.
  - Otherwise the VM writes it, so any writable direct i16 cell works.
- **H-5: `return a != b` keeps its operator.**
  - It lowers to the `!=` compare, as written. That is not Maxis's `if a == b { return false }` polarity, and it is not a regression to undo silently.
  - Sources that need Maxis's node write `return !(a == b)`, which lowers to `==` with the exits swapped.

### 2. Tier 1: compile-time branch specialization (`src/v2/specialize.rs`)

- **Syntax.** `const NAME = match s { P => v, .., _ => break };`, `const NAME = if c { a } else { b };`, and the tuple forms `const (A, B) = ..;` bind names to compile-time operands chosen by a runtime test.
- **Expansion.** The parser expands each binding when it closes the block. The statements that depend on it (its *region*) are copied into each arm with that arm's values substituted.
- **Cost.** No local holds the choice. An operand that must be a constant (a GUID, a string index, an animation, a BCON entry) can vary by branch.
- **Inline operands.** A `match` or `if` used directly as a builtin's operand is split the same way by the lowering; its region is that one statement.
- **Soundness rules.**
  - Arms yield compile-time operands or diverge: `break`, `continue`, `return`, or a block ending in one.
  - A `match` needs `_`, and an `if` needs `else`.
  - The region runs up to the last statement that names a bound name. It extends to every later use of a `let` it declares.
  - The region may not:
    - redeclare a bound name;
    - write a name an arm yields;
    - contain a `defer`;
    - name a bound name inside `raw { }`.
- **Other passes.** `fmt` and `lint` are token- and AST-based and see ordinary statements, so they need no change.

### 3. Tier 2: zero-cost inline helpers

Inline `fn` helpers lower directly into their caller's targets:

- **Returns.** An early return is a `break` out of the helper's single-pass loop, or the caller's arm or tree exit, with no `__fn_ret` flag.
- **Bool and `?`.** A bool or `?` helper routes the caller's branch directly, with no `__ret` local.
- **Values.** A value helper stores into the caller's destination.
- **Read-only arguments.**
  - They are substituted in place, with no `__arg_*` snapshot, when the cell is proven unchanged.
  - A possible alias, a primitive that clobbers, or a Sim-side `stack.attr` read keeps the snapshot.
- **Helper locals.** Disjoint helper locals share a word unless the tree formats text (H-3).
- **Types.** `Guid` parameters and `fn -> Result` helpers inline.
- **Fallback.** Cancellation, an exit from a caller loop, and layout-sensitive code keep the general path.

### 4. Tier 2: stack cursor

- **`for … else`.** All five object iterators take an exhaustion block. Exhaustion reaches it directly, and `break` skips it.
  - This removes the Fridges 4112 flag.
- **In-place stepping.** `while set_to_next(guid, K, dest: stack_obj) { .. }` steps the cursor in place.
  - It starts at the current object and needs no locals.
  - Modes `category` (6, reads `param[0]`) and `on_tile` (8) are available.
  - `part` gives a single multipart step.
- **`create_object` into the stack object.** `if create_object(K, dest: stack_obj) { .. }` branches on the primitive itself, with zero save locals.
- **`self` stays the entry object.**
  - In a Sim tree that moves the cursor explicitly, `self` is still the entry object.
  - Temporary moves for `self` restore the active cursor.
  - `refresh(target: self)` there is refused.

### 5. Tier 2: `const` inside `raw { }`

- **Constants.** A `const` in an operand position of raw `.air` is its number; a GUID is written in hex.
- **Tuning entries.** A tuning table's `Table.entry` is its `bcon[N]` operand.
- **Left alone:**
  - labels (`@x`) and branch targets;
  - fields, cells, call names and argument keys;
  - strings and comments;
  - `.air` keywords.
- **Raw locals win.**
  - A name any raw block of the tree declares (`local x`, `let x`) stays that local in every raw block, since `.air` locals are tree-wide.
  - Declaring a name that an earlier raw block read as a const is an error, not a silent change of meaning.
- **Why this is safe.** A bare name that is not declared was already an `.air` error, so no previously valid raw code changes meaning.

### 6. Tier 2: `@slot(N) let`

- **Syntax.** `@slot(N) let x = ..;` and `@slot(N) let x: T;` put `x` in frame word `local[N]`. This is how a source matches a Maxis frame that reads a word only some paths write.
- **Pipeline.**
  - Through the optimizer the variable is an ordinary local named `__slot<N>_x_k`, which never merges with another.
  - When the tree is finished, `opt::assign_frame_slots` rewrites it to `local[N]`.
  - It also places the tree's other generated locals first-fit, in declaration order, around every slot `.air` reports as explicit. On its own, `.air` would put them after the highest explicit slot.
  - The `.air` scan (`lower::air_local_usage`) now records explicit slots as a set, not a maximum. The allocation of trees without `@slot` is unchanged.
- **Kept as `.air` places them:**
  - locals a raw block declares under its own name;
  - every unpinned local of a tree with raw `op` bytes.
- **Refused:**
  - inside an inlined helper, because the frame is the caller's;
  - in a tree with a pinned `push_interaction` object slot;
  - while another variable of the same slot is in scope.
- **Allowed:** disjoint scopes may reuse a slot, and a raw block may share a pinned word on purpose.
- **Formatting.** `fmt` does not know the attribute yet and prints `@ slot(N)`, which parses the same. Teaching `fmt` the attribute is a one-word follow-up in `fmt.rs` (attribute list) for the lint track.

### 7. Tier 2: `@frame(...)` on lifecycle blocks

- **Syntax.** `init`, `main`, `load`, `reset` and `cleanup` take `@frame(args, locals, format, revision, kind)`, as a standalone tree does. One header writer serves both.
- **Effect.** An original main that declares four arguments is now `@frame(args: 4) main { }`. Before, it was a standalone tree, plus a Main stub the splicer had to drop.
- **Parameters.** `param[0..args]` becomes addressable.
- **Refused:**
  - `@frame(empty)` on a lifecycle block;
  - a second `@frame` on the same lifecycle tree, from a template and its object.
- **Init with no prelude.** States declared at an explicit slot without an initializer add no init prelude. An init can therefore match an original's exactly.

### 8. Tier 2: context polymorphism

- **Runner cells.** `me.person[i]`, `me.motive[i]`, `me.data[i]`, `me.attr[i]` and `me.slot[i]` are the cells of whoever runs the tree.
  - They lower to `my.*` in an object tree and in a Sim tree alike.
  - A binding the source names `me` keeps its meaning.
- **`@context(inherit)`.** A standalone tree marked this way can be called from object trees and Sim trees.
  - A private call keeps the caller's runner and stack object, so the callee needs no wrapper and no extra locals.
- **Soundness by construction.**
  - The compiler lowers an inherit tree once in each context and requires the two `.air` texts to be identical.
  - The bytecode therefore means the same thing whichever context calls it.
  - `self` and `sim` are refused, because they differ by context. Only `me.*`, `stack_obj`, parameters, temps and globals pass.
  - The check restores the dialog-string table between passes, which is the only interior-mutable state in lowering.
- **Deferred.**
  - A typed `self` on a caller-supplied stack object needs a call-site proof of its identity and type.
  - Changing the runner needs a VM operation that does so.

### 9. Deferred to PR I

Each item below is clean as its own change, so this PR leaves it out:

- **Compound arithmetic into the target cell.** `self.n = a + b * 2` still goes through a temporary when the destination is not a local.
  - Literal folding (`40 + 1`) belongs with it.
- **`sound foo = fwav("name", id: N);`** A sound declared inline from its FWAV.
- **`use` and `import` for shared consts, and `extern mod globals`.** These need a module-resolution design (visibility, cycles, pack layout) of their own.
- **`fmt` support for `@slot`.** Owned by the lint track; see decision 6.

## Consequences

- **Pilot gaps closed.** The R2 reports' entry-save bugs (1–3) and the `!=` polarity question (4) are closed. Most raw sites listed under stack cursor, `create_object`, `relationship(value:)`, `@slot`, Init/Main stubs and person data in object context now have v2 spellings.
- **Verifier gap 1 remains.** Cut-point bisimulation compares local slots literally. `@slot` lets a source match Maxis's slots exactly where that matters.
- **New invariant.** Generated local names starting `__slot` are reserved for pinned locals. `merge_group` and the text liveness treat them as source locals.
