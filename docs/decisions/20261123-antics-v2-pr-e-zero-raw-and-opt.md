# `.antics` v2 PR E: Final Zero-`raw` Gaps, Expression `match`, Multi-Variant IFF, and Backend Optimization

**Date:** 2026-11-23  
**Branch:** `antics/v2-pr-e-zero-raw-and-opt`  
**Track:** Track 8 (`.antics` v2 Language Ergonomics & Extensions — PR E)

## Context

A second behavioural-equivalence pilot rewrote a 51-tree commercial object in v2 against PR D. As before, it ran locally and nothing from it is in the repository. It brought raw blocks down from 113 sites to 12. Every remaining site was one of nine v2 gaps:

1. No spelling for the stack object's dynamic-sprite flags.
2. `param[n]` could not be assigned.
3. Another object's slots (`stack.slot[i]`) could not be read.
4. `run_functional` only took named kinds.
5. `push_interaction` could not take its object from a local.
6. (`animate` event local outside the frame; already expressible since PR D's `animate |event|`.)
7. Other Sims' person data could not be read from a `lot.sims()` loop.
8. No `inc_lt` / `dec_gt` on temps and fields.
9. `notify_out_of_idle()` padded with `0x00` where the original pads `0xFF`.

It also listed six lowering losses that cost proofs, locals, or nodes:

- **L1:** the dead `__self_obj` save;
- **L2:** `!(a > b)` polarity;
- **L3:** `match` does not fold;
- **L4:** `__cond` locals;
- **L5:** heavy `create_object` / `let Some` lowering (25 nodes / 2 locals became 67 / 15);
- **L6:** `.air` places named locals after any literal `local[i]`.

PR E closes the gaps and the losses that the compiler can fix soundly.

Constraints:

- **Existing `.air` syntax only.** Everything lowers onto it. Nothing changes in the `.air` parser, lowerer, verifier, or `compile.rs`. Track 7 works on `src/verify*` and Track 9 on `src/v2/{fmt,decompile}.rs` in parallel.
- **Synthetic fixtures only.** Every test fixture and example is synthetic.

## Decisions

### 1. Zero-`raw` gaps

- **`push_interaction` from a local.**
  - `push_interaction(object: x, ...)` takes any object-handle expression, and so does the method form `x.push_interaction(menu: ..., ...)`. An integer literal still means a parameter index, as before.
  - The VM reads such an object from a *local slot index* (flag `0x02`). `.air` cannot name a local there, so v2 copies the handle into an explicit `local[k]` right before the push: `local[k] := x; push_interaction(..., object: k, flags: 2)`.
  - **One pinned slot per tree: `local[0]`.** `.air` allocates every named local after the explicit ones, so the slot never collides with a binding.
  - **No `raw` in the same tree.** Raw `.air` can name local slots in forms no text scan sees (`scope(25, N)`, raw `op` operand bytes, primitive fields). A tree that pushes from a handle therefore may not also emit a `raw { }` block, in either order. Raw code in a folded-away branch emits nothing and does not count. A constant parameter index still works there.
  - Flag `0x02` is implied; spelling it as well is an error.
  - Pinning the binding itself (no copy node) would need `.air` to place a named local at a fixed slot. That is out of scope here, so the copy costs one node per push.
- **Fields of other Sims and objects.**
  - **Object-handle bindings** (`for p in lot.sims()`, `lot.objects()`, `ObjectId` locals and params) read the same Sim fields that `sim.` has:
    - `p.person[i]`, `p.age` (58), `p.priority` (33), `p.skill.<name>`;
    - `p.is_visitor` (person word 32);
    - `p.kind` (person word 65);
    - `p.motive.<name>` / `p.motive[m]`;
    - `p.slot[i]`, `p.hand`, `p.room`.

    Each read switches `stack_obj` to `p`, reads the `stack.` cell, and restores `stack_obj`, exactly like the existing foreign-attribute reads. `p.person[i]` (and its named forms) may also be assigned.
  - **Slots.** `self.slot[i]` reads the object's own slot: `my.slot[i]` in object trees, `stack.slot[i]` in Sim trees. Slots are read-only.
  - **Dynamic sprites.** `obj.sprite_flags[i] = v` (and its read), with `set_dynamic_sprite(index: i, value: v)` as the statement spelling for `self`.
    - These lower to the VM's scope 27, which takes the flag index *from a temp*: `temp[0] := i; scope(27, 0) := v`.
    - They clobber `temp[0]`, as the original's own spelling does. A value that reads `temp[0]` is snapshotted before the index is written.
    - `&mut obj.sprite_flags[i]` passed to a helper snapshots a run-time `i` at the call, like the object handle. The reference keeps naming the cell the call chose, even if the helper reassigns `i`.
    - The target becomes `stack_obj` for the access, like any foreign access.
- **`run_functional` by number.** `obj.run_functional(4, change_icon: true)` accepts an integer function index (0..=255) as well as a kind name. A numeric index is always untyped: no signature is checked, and `let Some` yields `i16`.
- **`notify_out_of_idle()`** lowers to `notify_idle() pad 0xff`, the original's operand spelling, which a verifier can observe.
- **Dead `__self_obj` save (L1).** Handled by the backend pass (§3): once the restores before `return` are known dead, an unread `__self_obj` disappears.
- **`param[n]`.**
  - `param[n]` is readable and assignable, including with `+=`, inside trees whose frame covers `n`:
    - standalone trees: positional params or `@frame(args:)`;
    - functionals: all 4 words of the frame the engine passes, whatever the functional declares. The functional's header declares `args:` as the larger of its parameter count and the highest `param[n]` it names, so `.air` accepts it.
  - Interactions and lifecycle trees have no arguments, so `param[n]` there is an error.
  - Named positional params stay immutable by name. `param[n] = ...` is the explicit escape hatch, and later reads of the named param see the new value.
- **`inc_lt(&mut x, n)` / `dec_gt(&mut x, n)`.**
  - These are condition builtins mapping one-to-one onto the VM's increment-and-compare operators (opcodes 11 and 17).
  - `x` must be a direct `i16` place: a mutable local, `self.<state>`, `self.graphic`, `self.data[i]`, `self.attr[i]`, `temp[i]`, `global[i]`, or `param[i]`.
  - They are not pure: `&&` / `||` never reorder them, and mutation analyses treat them as writes.
  - A counted loop is `while inc_lt(&mut i, 5) { ... }`: one node.

### 2. Ergonomics

- **Expression `match`.** `let x = match s { A => e1, B => e2, _ => e3 };`, `return match ...;`, and any other value position accept a match.
  - **Arms are expressions.** All arm values must have exactly the same type. Assignment compatibility is not enough: `0` and an `ObjectId` do not mix, and neither do a generic and a typed handle.
  - **Strictly exhaustive.** Enums must cover every variant or have `_`. `i16` needs `_`, or literal and range arms that together cover `-32768..=32767`.
  - **Nesting.** A match expression counts as two levels against the parser's depth limit of 64, so `match match ... x {..} {..}` stops at 32. Each level costs two expression parses of stack, and the budget has to fit the default 2 MiB test-thread stack.
  - **Arms are values, not blocks.** `A => { ... }` is an error in expression form. A bool-valued arm lowers into the result local like any other bool expression.
  - **Lowering.** The match lowers to an if/else-if chain into one result local. The last arm is the `else`, with no test.
  - **Statement form unchanged.** The statement form keeps its rules: non-exhaustive `i16` matches remain allowed there.
- **Constant `match`** (L3). A scrutinee that is a compile-time constant (literal, enum variant, `const`, or a helper argument that lowered to a literal) selects one arm. A tunable template `param` is a BCON read, so it is never folded, not even through a helper argument. The other arms are still lowered into the discarded buffer, so they stay checked. Statement and expression forms both fold.
  - **No scrutinee copy.** A runtime scrutinee that is a single operand with no side effects (local, param, field, BCON) is no longer copied into a `__match` local. An arm body could change that operand, but the chain tests it only before any arm body runs.
  - **One arm still reads.** A match with a single arm and no test (only `_`, or one arm that covers everything) still reads its scrutinee once, into a `__match` local, unless it is a local or a literal. A cell read can fault, and dropping it would hide that.
- **Multi-variant IFF.** An object may declare catalog variants:

  ```
  variant "Budget Model" { guid: 0x7F40_0101, price: 120, init: init_budget, description: "..." }
  ```

  - **Shared pool.** Each variant is one more OBJD (and OBJf and CTSS) in the same IFF, sharing the BHAV, BCON, STR#, TTAB, SLOT, and graphics pool.
  - **Overrides.** A variant overrides the GUID, price, catalog name and description, and the `init`/`main` trees. Those trees are named by standalone `tree` identifiers.
  - **How it is built.** `compile.rs` is out of scope, so the variants are appended by `v2/mod.rs` after compilation:
    - the primary OBJD's words are copied with the GUID (14/15), price (18), catalog-strings id (41), init (43), and main (5) words patched;
    - its OBJf is copied with entries 0/1 replaced;
    - the IFF is re-encoded with a fresh `rsmp`.

    Variant OBJD/CTSS ids continue after the primary's and must be free.
  - **Restrictions.**
    - Variants are rejected on multi-tile objects (tiles already own the OBJD group).
    - A variant needs a `guid`. Its name must differ from the primary's and from other variants', and its GUID must be new.
    - A variant's `init:` / `main:` must name an object-self `tree` with no parameters and a frame of at most four words, because the VM runs it as the object's init or main with four zero arguments.
  - **Pack GUID checks.** `compile_v2_pack_dir` reads every OBJD GUID in each compiled IFF, so it includes variant GUIDs. The CLI's `compile_pack_dir` (in `compile.rs`, outside this PR) checks only `guid` and `tile_guids`, so it does not yet see a variant GUID that collides with another object's. Fixing that is a one-line follow-up in `compile.rs`. Putting variant GUIDs into `tile_guids` is not an option: `sim_pack` treats a non-empty list as multi-tile.
  - **`init:` replaces the whole init.** A variant's init tree runs instead of the primary's, state defaults included. Leave `init:` out to share the primary's.
  - **Compatibility.** Single-variant source is unchanged byte-for-byte.

### 3. Backend optimization (`src/v2/opt.rs`)

The lowering stays simple. A new pass rewrites each emitted `.air` tree body before it is returned.

- **What it works on.** It parses the structured text v2 emits: one simple statement per line, plus `if` / `else if` / `else` and `while` blocks (`while true` is `.air`'s condition-free loop).
- **When it skips a tree.** Lowering never hands it a tree whose bytes matter beyond `.air` semantics:
  - a tree that emits a `raw { }` block (its literal calls and ops can read the next instruction's bytes, and it can name local slots);
  - a tree with a literal-argument call to a callee whose frame reads more than four words. The VM copies the extra words from the instructions after the call, so removing or renumbering those would change the arguments. An `extern` is trusted to declare its callee's whole frame.

  The pass itself also leaves a tree untouched when any of these holds:
  - the tree has labels, explicit branch targets (`->`), or comments (`//`, `/* */`);
  - a line holds more than one statement or is not a recognised block line (some `raw { }` blocks);
  - a line can name a local *slot* by number: a raw `op` (other than the operand-free ops v2's builtins emit for find best interaction, grab, drop, and gosub found action), or `scope(s, …)` whose scope is 25 (locals) or not a literal. Coalescing and dropping named locals renumber slots, which would silently retarget such a reference.

**Facts the rewrites rely on.**

- An expression node that writes a local or `stack_obj` has no other effect.
- `stack_obj` is per frame: a callee's changes are not copied back, and `return` pops it.
- The last result is read only by primitives. So a rewrite that removes a node requires the next node that runs to be an assignment, a `let` with an initializer, a `return`, or an `if` / `while` whose test is an expression compare (no primitive call).
- A `let x;` without an initializer emits no node.

**The rewrites**, all exactly semantics-preserving, repeated until nothing changes:

- **Unreachable code.** Statements after an unconditional `return`, `break`, or `continue` in the same block are dropped. The declarations they held stay as `let x;`.
- **Single-pass loops.** `while true { B; break; }` with no other `break` / `continue` of that loop in `B` becomes `B`. The loop has no node, and the `break` went where `B` now falls through.
- **Dead `stack_obj` stores** (L1, L5).
  - A store directly before `return` is dead.
  - A store directly before another store is dead when the second store does not read `stack_obj` (its value is a local, a literal, or `my.id`, not a `stack.` cell).
  - A store is redundant when straight-line code proves `stack_obj` already holds that value: an earlier `stack_obj := x`, `let x = stack_obj`, or `x := stack_obj` with no write to either in between. Any write to `stack_obj` (`+=` included) or to `x` ends that knowledge. A redundant store is dropped under the next-node rule.
- **Dead local stores.** Liveness runs over the structured tree (if/else chains, loops to a fixpoint, `break` / `continue` / `return`). A plain store (`let x = e;` / `x := e;`) to a v2 local is dropped when nothing reads the local afterwards, under the next-node rule. Its operand must be one that cannot fault: a named local (v2 writes each before reading it), an integer literal, `stack_obj`, or `my.id`. Cells of objects, temps, params, and globals stay, since an index past the data or the frame faults. So does an explicit `local[N]`, which can be unwritten. This removes the unread `__self_obj` save once its restores are dead, and helper return flags.
- **Copy forwarding.** `t := e; d := t;`, with `t` a v2 local dead afterwards, becomes `d := e;`. It is one expression node where there were two, with nothing between the read of `e` and the write of `d`.
- **Duplicate tails.** `if c { A; T } else { B; T }` becomes `if c { A } else { B } T`, where `T` is a maximal run of identical trailing simple statements. Paths that leave early skipped `T` before and still do. Paths that fall off either branch run `T` once, after the same node as before.
- **Local coalescing (L4/L5).**
  - **Mentions.** Every mention of a local counts as a use, including primitive operands such as `dest:` and `local:`. Only `let x = e;` and `x := e;` kill `x`. So a primitive that writes a local is treated as reading it too, which is conservative.
  - **Interference.** Two locals interfere when one is mentioned at a point where the other is live in or out.
  - **Who may merge.**
    - Compiler temps (`__and_`, `__match`, `__mval`, `__fn_ret`, `__arg_`, `__rf_`, `__fdata`, …) merge only with other compiler temps.
    - A source binding merges only with another binding of the same source name (`__v_i_3` with `__v_i_7`, the `for i` loops one after another).
    - Locals live at tree entry never merge, and neither do `__self_obj` and `__saved_stack_obj`.
  - **How.** The merge is greedy in declaration order, so the result is deterministic. A merged `let y = e;` becomes `x := e;`, which is the same node.
- **Tidying.** `let x;` declarations nothing names are dropped, and `let x;` directly before `x := e;` is spelled `let x = e;`.
- **Text handling.** Token scans blank string contents byte for byte (so multi-byte text keeps offsets), and a name after `.` (`bcon.__t_2`) is a field, never a local.
- **Not done here.**
  - *Jump threading:* the `.air` compiler already resolves a branch to a following `return` into the return code.
  - *Explicit slot naming:* `.air` text cannot name a local by slot. v2 has no way to put a local into dialog text, so a `$Local` token in a string table cannot rely on v2's local numbering, before or after this pass.

**Lowering changes that pair with the pass.**

- **Helper arguments.** A plain caller local passed by value to a helper is used in place, with no `__arg_*` copy. This needs the helper never to mutate or borrow that parameter, and no argument of the same call to borrow the local.
- **`!(a > b)` polarity (L2).** A written `!(cmp)` now emits `.air`'s negated compare (`!a > b`): the same compare node with its branches swapped, as the original builds it. Before, it was normalised to `a <= b`.

**Effect on the open pack** (20 objects, BHAV totals from the compiled IFFs):

- **Nodes:** 866 → 785 (−9.4%).
- **Locals:** 122 → 63 (−48.4%).
- **Equivalence.** Every object still compiles and passes `cheats_cache_and_open_pack`. `sims-antics verify-equiv --all`, old build against new, finds no DIFFERENT tree. The rest are EQUIVALENT or INCONCLUSIVE, for these reasons:
  - Smaller frames move callee frame offsets.
  - Dropped `stack_obj` stores change the event trace.
  - Some loops are unbounded.
  - Some trees have more than 4096 paths.

## Consequences

- **Pilot coverage.** The pilot's nine gaps have raw-free spellings, and `push_interaction` helpers can be factored, because nothing needs raw `return` paths.
- **Two-step output.** The v2 output is now "lower, then optimize". Tests that pin exact `.air` text see the optimized form.
- **Track 9.** `V2Expr::Match` (the expression form) and `ObjectMember::Variant` are new AST variants. The formatter and decompiler (Track 9, `src/v2/{fmt,decompile}.rs`) need arms for them when they land.
- **Variants without `compile.rs`.** Multi-variant IFFs are built without touching `compile.rs`. Moving them into the manifest later would be a pure refactor.
