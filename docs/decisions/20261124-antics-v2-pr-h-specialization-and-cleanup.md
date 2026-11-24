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
  - The `.air` parser (`src/parser.rs`) gains tree-level named constants (`const NAME = value;`) and a lenient parse for shape-only analysis (decision 5).
  - The `.air` tree header gains `min_locals: N` (decision 6): the frame holds at least `N` locals, and named locals go after them. Previously valid `.air` keeps its meaning.
  - `src/v2/raw_facts.rs` is the one parsed analysis of raw `.air` that lowering and specialization share.
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
  - A relationship read without flag 1 adds a missing row, so it writes on both exits. With flag 1 a missing row takes the false exit unwritten.
  - Raw code is split one statement per line before the analysis.
  - Positional branch targets block the drop, wherever their lines fall after the `->`. A raw `local` declaration still keeps the save.
  - Raw code the analysis cannot parse (a label graph) keeps the save when it moves any named local.
  - In a tree that keeps its `.air` layout, a tree that formats text keeps the save, since `$Local:N` could name its word.
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
- **Inline operands.** A `match` or `if` used directly as a builtin's operand is split the same way by the lowering; its region is that one statement. Only statement positions split: a builtin used as a condition (`if create_object(if .. { A } else { B }, ..)`) reports the operand error, and a named `const` binding spells it.
- **Soundness rules.**
  - Arms yield compile-time operands or diverge: `break`, `continue`, `return`, or a block ending in one.
  - A yielded name must not be a variable (any name a `let`, parameter or loop binds earlier in the file), and an arm cannot yield `stack_obj`, which primitives change. A field's root counts: `X.graphic` with `X` a variable is a variable read. A variable's value can change inside the region, so it is not a compile-time operand; a run-time choice uses a `let` with `match` or `if`.
  - A `match` needs `_`, and an `if` needs `else`.
  - The region runs up to the last statement that names a bound name. It extends to every later use of a `let` it declares.
  - The region may not:
    - redeclare a bound name;
    - declare a name an arm yields (passing it to a call reads it);
    - contain a `defer`;
    - name a bound name inside `raw { }`;
    - assign or borrow a bound name (substitution would write through it);
    - hold a `raw { }` block that defines a label, declares a local, or branches to a numbered node (each arm's copy would define it again, or jump into the first copy).
  - In a tree that formats text, the region may not declare a variable: each arm's copy takes its own frame word, so no `$Local:N` names it on every path. Declaring it before the binding works. Lowering detects the copies by declaration span per inline call site and defer instance.
  - A tree formats text when it runs a text primitive (Dialog 36, Action String 50) in any spelling: a builtin, raw `.air` parsed by `raw_facts` (`action_string (..)` with a space, raw `op 50`), or an inlined helper's copy.
- **Inline operands** split only when the selector is pure (no call, no `&mut`), so no operand read before it moves after a write it makes.
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
- **Fallback.** Cancellation, an exit from a caller loop, and layout-sensitive code keep the general path. So do:
  - a helper with a cleanup (a `defer`, or the unlock of `with lock`), in value and in bool position (its cleanup would run between the condition and the caller's arm, which may read the last result), and a result stored into a pinned `@slot` word. The result is staged and stored after the helper's cleanup, which may read the word as `$Local:N`. A fault ends the frame, so a partial store before one is unobservable, as in ordinary code;
  - a `return` in a `for … else` exhaustion block, which is lowered inside the iterator loop and needs the return flag.
- **Pinned arguments.** An argument in a `@slot` word is always copied. A helper's raw code can write that word by number.
- **Raw helpers.** When the helper, or a helper it or an argument calls (transitively), runs raw code, every by-value local argument is copied: raw code can write any frame word by number or by its `.air` name.

### 4. Tier 2: stack cursor

- **`for … else`.** All five object iterators take an exhaustion block. Exhaustion reaches it directly, and `break` skips it.
  - This removes the Fridges 4112 flag.
  - Exhaustion restores the stack object only when the iterator moves it (a stack cursor) or an iteration may leave it moved: the lowered body is checked (`opt::body_may_leave_stack_obj_moved`) for a path to a `continue` or its end where a write is not followed by the restore. A write is any `.air` token sequence that names the stack object (`stack_obj`, or `scope(10, ..)` in any spacing) as a target: an assignment, any arithmetic operator (`sqrt`, `inc_lt`, ..) or a `dest:`. A comparison or `flag_test` only reads it. A subroutine call (`call N(..)`, `call name(..)`, `call "name"(..)`, `subroutine(N)`) is not a write: the VM gives each frame its own stack object. The paths follow the control flow: a nested loop's `break` carries its state out of that loop, a `return` ends its path, and code no path reaches counts for nothing. A field read through the item moves the cursor and back, so it needs none. A redundant store would replace the iterator's false result, which the block can read (`gosub_found_action`). Where a restore is needed it is bookkeeping, as with the entry save.
  - Loop exits inside a raw block are found by parsing it (`raw_facts`): a raw loop's own `break` and a commented `continue` do not exit the `for`. A block that names a constant only an earlier raw block declares gets the shape-only parse. If that also rejects it, the loop-exit facts fall back to its tokens (`break`, `continue`, `->`), and the other facts stay conservative.
  - A raw `break` or `continue` cannot leave a loop when that edge does work: a `continue`'s counter or range step, or a `break`'s restore in a `for … else`. The raw jump would skip it; v2 `break`/`continue` run it. Each edge is judged on its own, so a raw `break` out of `repeat` stays valid.
  - Precision matters both ways: a missing restore leaves `self` on a moved object, and a needless one replaces the last result the exhaustion block may read.
- **In-place stepping.** `while set_to_next(guid, K, dest: stack_obj) { .. }` steps the cursor in place.
  - It starts at the current object and needs no locals.
  - Modes `category` (6, reads `param[0]`) and `on_tile` (8) are available.
  - `part` gives a single multipart step.
- **`create_object` into the stack object.** `if create_object(K, dest: stack_obj) { .. }` branches on the primitive itself, with zero save locals.
- **Loop conditions re-read.** A condition that reads a `self` cell after the cursor moved swaps the cursor and reads at the head of every iteration. It is never hoisted.
- **`self` stays the entry object.**
  - In a Sim tree that moves the cursor explicitly, `self` is still the entry object.
  - Temporary moves for `self` restore the active cursor.
  - `refresh(target: self)` there is refused.

### 5. Tier 2: `const` inside `raw { }`

- **`.air` named constants.** Raw `.air` keeps its own grammar: lowering lexes each raw block, and every v2 `const` (or tuning `Table.entry`) it names becomes a tree-level `.air` declaration, `const NAME = value;` (a GUID in hex, a tuning entry as its `bcon[N]` operand).
- **Where they resolve.** The `.air` parser resolves a named constant only where it takes an operand, an integer, a local reference (`animate(.., local: L)`) or a string index (`dialog(message: K)`, also dotted). Labels (`@x`, bare `x:`), branch targets (also after comments or on later lines), mode words such as the `part` of `set_to_next`, fields, call names, argument keys, strings and comments keep their meaning, because the parser never looks a name up there. No text rewrite and no list of `.air` words is needed (the round-1 `air_words.rs` list is gone).
- **Locals win.** A name the tree declares as a local (`let x`, `local x`, tree-wide in `.air`) is that local everywhere in the tree, constant or not. A constant is read-only: its value must be an integer, a literal (scope 7) or a `bcon[..]` entry (scope 26), so it can never name a cell. Every position the VM writes rejects it (dotted or not): an assignment's left side, `expr`'s left side under a writing operator (not a comparison or `flag_test`), `random`'s and `get_direction_to`'s destination, `dest:`, `relationship(value:)` without flag 4, and the left side of `inc_lt`/`dec_gt`. Declaring a name again with the same value is accepted (a raw block inlined twice); a different value is an error.
- **Only live code and real names.** Only raw blocks in code lowering keeps register constants or declared locals: a `local K` in discarded code (`if false`) declares nothing. A constant named like an `.air` keyword (`temp`, `local`, ..) is never emitted, so it cannot shadow the keyword. One constant used twice in a tree (an inlined helper) is declared once. A raw block's own `.air` constant (`const K`, `const T.e`) is its own, by full name: v2 declares no second one for that block, and it is no local (it moves no `@slot`). Discarded code never clashes with a live constant.
- **One meaning per tree.** One name must name one value in a tree: an inlined helper's raw code may see another module's constants, and a clash is an error.
- **Why this is safe.** A bare name that is not declared was already an `.air` error, so no previously valid raw code changes meaning.

### 6. Tier 2: `@slot(N) let`

- **Syntax.** `@slot(N) let x = ..;` and `@slot(N) let x: T;` put `x` in frame word `local[N]`. This is how a source matches a Maxis frame that reads a word only some paths write.
- **Pipeline.**
  - Through the optimizer the variable is an ordinary local named `__slot<N>_x_k`, which never merges with another.
  - When the tree is finished, `opt::assign_frame_slots` rewrites it to `local[N]`.
  - It also places the tree's other generated locals first-fit, in declaration order, around every slot `.air` reports as explicit. On its own, `.air` would put them after the highest explicit slot.
  - The `.air` scan (`lower::air_local_usage`) now records explicit slots as a set, not a maximum. The allocation of trees without `@slot` is unchanged.
- **Provenance.** Only the locals lowering itself pinned (a map from `.air` name to word) are placed: a raw block's own `__slot..` name is an ordinary raw local, and a `@slot` in discarded code pins nothing.
- **Rewriting.** The rewrite works on `.air` tokens and changes local operands only. A label, a branch target, a field, an argument key, a call or subroutine name (`call x(..)`, `x(..)`, `subroutine(x)`) or a `const` name spelled like a pinned name stays what it is.
- **Stores are kept.** The optimizer never drops, forwards or undeclares a pinned local's store: another tree reusing the frame, or raw code, can read the word by number.
- **Inside the frame.** With `@frame(locals: L)`, `@slot(N)` needs `N < L`. Without it, the tree header asks for `min_locals: N + 1`, so a slot nothing names again (an uninitialized one a raw primitive or text reads by number) stays in the frame.
- **Kept as `.air` places them:**
  - locals a raw block declares under its own name (even one starting `__`);
  - every unpinned local of a tree with raw `op` bytes.
- **Limit: raw `op` bytes.** Raw `op` bytes that address a frame word by number are opaque to placement. An unpinned local may share that word, as with `$Local:N`: only `@slot` fixes a number a raw site relies on.
- **Refused:**
  - inside an inlined helper, because the frame is the caller's;
  - in a tree with a pinned `push_interaction` object slot;
  - while another variable of the same slot is in scope, including a shadowed one a `defer` still captures.
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
- **`$Local:N` contract.** In a tree that formats text, each source local keeps a word that holds its value. The word's number is fixed only by `@slot(N)`; an unpinned local's number is the compiler's.
- **Review.** Codex Sol review round 1 found twelve soundness bugs and round 2 fourteen more, plus an architecture finding: raw `.air` was analysed by text approximations of its grammar. Round 2 replaced those with the `.air` parser (named constants) and one parsed analysis (`raw_facts`). Round 3 found the remaining edges of that design: constants in duplicated, discarded or keyword positions, cell-valued `.air` constants, slot provenance in discarded code and call names, and the token forms of `stack_obj` writes. Round 4 made the restore analysis follow `break` and `return` paths, closed raw loop exits that skip loop work, the `with lock` cleanup of bool helpers, constant writes outside assignments, string-index constants, raw-owned constants and the frame floor of an unnamed `@slot`. Round 5 narrowed those fixes where they were too broad: reads (`expr` comparisons, `flag_test`), per-edge loop work, full-name ownership of raw constants, unreachable code, quoted call targets, and `relationship`'s output operand. Each fix has a synthetic regression test in `tests/v2_pr_h_review.rs`.
- **Random ranges.** `random` writes its destination only when its range is a literal: a range resolving to a word pointer (global 34) draws into the range cell, so the entry-save analysis does not count it as a write.
