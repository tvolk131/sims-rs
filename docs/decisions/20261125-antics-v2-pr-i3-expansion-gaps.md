# `.antics` v2: PR I-3 Expansion Gaps

*Decided 2026-11-25.*

**Branch:** `t3/antics-v2-pr-i3-expansion-gaps`  
**Track:** Track 8 (`.antics` v2)

## Context

The expansion-pack pilots (Bonehilda, Car Portal, Servo, Bull, DiningBooth, Snowman) kept raw
`.air` blocks where v2 had no spelling for what Maxis wrote. There were two causes:

- no v2 spelling for Remove Object of anything but `self`, Find Best Object as a branch,
  Set To Next into a local or a parameter, and the definition and job-data cells;
- two optimizer rules that changed what such a tree observably does:
  - a trailing `stack_obj = saved;` was dropped;
  - a constant `global[NAME]` counted as a global-34 read.

Each new spelling compiles to the node Maxis wrote, with no extra node or local. The test is a
synthetic object that uses every new form: built once with the new forms and once with the same
trees as raw `.air`, it gives byte-identical IFF, and `verify-equiv --all` reports EQUIVALENT.
The meaning of each cell and primitive was read from `sims-vm`, named per item below.

Every regression test is synthetic and lives in `tests/integration/v2_rust_syntax.rs`.

## Decision

The code is the definition of each rule. This section says what changed and why.

### 1. `remove_object(target)` / `target.remove()` (`lower/actions.rs`)

Remove Object Instance (opcode 18, `sims-vm` `primitives/remove.rs`) names its object by mode,
not by value: 0 is the running object, 1 the stack object, 2 object 0. A missing object takes
the false exit.

- **Target:** `me`, `stack_obj`, a handle the stack object holds (a cursor loop's binding),
  or `self` where `self` is one of those.
  - `self` is `me` in an object's own trees.
  - `self` is the stack object in a Sim tree, as for `destroy_self()`.
  - In a Sim tree, `self` after `stack_obj = x` is the new stack object (PR D's rule). The
    `stack_obj_clobber_after_self` lint flags that use.
- **Refused targets.** Each gets an error that says what to write instead:
  - any other handle: select it into `stack_obj` first;
  - `self` in a `context = inherit` tree;
  - `self` in a Sim tree that steps the cursor (`destroy_self()` selects it there).
- **Options:** `flags:`, `no_yield:` and `skip_resets:`, shared with `destroy_self()`.
- **Where it works:** as a statement, a condition (`if !x.remove() { .. }`) and a bool value.

### 2. `find_best_object(kind)` as a condition (`lower/cond.rs`, `lower/ast_facts.rs`)

The untyped search in a condition is Find Best Object's own node (`sims-vm`
`primitives/find_best_object.rs`):

- the true exit leaves the found object in `stack_obj`;
- the false exit leaves `stack_obj` as it was.

A tree that uses it counts as one that steps its own cursor (`ExplicitStackOps`), like
`set_to_next`. So later builtins that move the stack object put back the found object, and a Sim
tree's `self` stays the object the tree started on.

The same holds in a `stack_obj.run_functional(..)` argument: the functional runs on the receiver,
and the found object is selected again after it. A helper `fn find_best_object` still wins over
the search, as it does in value position.

`let Some(x) = find_best_object(..)` already restores the cursor itself and is unchanged. The
typed `find_best_object::<T>()` binds a checked object, so as a condition it is refused, with a
pointer to the `let Some` form.

### 3. `set_to_next(.., dest: x)` steps a named cell (`lower/actions.rs`)

Set To Next (`sims-vm` `primitives/set_to_next.rs`) reads the cursor from its destination and
writes the next one back. Operand bit 7 selects the destination scope and index; without it the
destination is the stack object.

- **Allowed destinations:**
  - `stack_obj`;
  - an `i16` or untyped `ObjectId` local;
  - `param[N]`;
  - `temp[N]`.
- **Refused destinations:**
  - an immutable parameter (the error names its `param[N]` word);
  - `temp[8]` and up (the VM bounds-checks the destination: a Sim has 8 temps);
  - a typed handle (the next object need not be of its type);
  - a bool, an enum, or any field.
- **A write:** the destination counts as a written local (`LOCAL_WRITING_BUILTINS`), so a
  helper's by-value parameter used as one is copied, not aliased to the caller's local.
- **The stack object:** stepping another cell leaves it alone (`set_to_next_steps_stack_obj`).
  Such a call is no cursor move, so a Sim tree keeps no entry save for it.
- **Already supported, now tested:**
  - `set_to_next(person, guid: X)` emits the GUID bytes, which person mode ignores;
  - beside's `local:` composes with `dest:`.

### 4. Definition and job-data cells (`lower/expr.rs`, `lower/tree.rs`)

Each is verified in `sims-vm` `memory.rs`. All are read-only. A write is an error, and so is a
`&mut` borrow.

| Spelling | Cell | Meaning (`sims-vm`) |
|---|---|---|
| `x.object_type` | `scope(21, 9)` | Definition word 9, the object type (2 is a person; `factory.rs` `word::TYPE`) |
| `x.sub_index` | `scope(21, 11)` | Definition word 11, a tile's place in its group (-1 the catalog master; `word::SUB_INDEX`) |
| `x.has_function[N]` | `scope(35, N)` | Whether the definition names a tree for function N (bool) |
| `job_field(F)` | `scope(33, F)` | Field F of the career in `temp[0]` at the level in `temp[1]` |

- **Which object.** Scopes 21 and 35 are cells of the stack object:
  - `stack_obj.<field>` (and `self.<field>` where `self` is the stack object) reads it in place;
  - any other object handle moves the stack object there and back, like any foreign read;
  - a state of the same name wins over the engine field;
  - a store to another object's cell copies a definition-cell value first, as it does a
    `stack.` cell (`reads_stack_obj`), since the store selects the other object;
  - a Sim handle (`sim`, or `me` in a Sim tree) is refused. It is read through `my.`, so a
    tree that never moves the cursor keeps no save to put it back with.
- **`job_field(F)`** reads the temps as they are where it is read. It is a plain operand, so
  `job_field(3) < me.person[10]` is one compare node, as Maxis wrote it.
- **Temp staging.** Compiler staging that writes temps now treats `scope(33, ..)` as a temp
  read and copies it first:
  - call arguments;
  - `run_functional` arguments;
  - `job_data`'s level;
  - a sprite-flag store.

### 5. A source `stack_obj = saved;` before the exit stays (`opt.rs`, `mir/pipeline.rs`)

The stack object at exit is observable:

- the popped frame's header (with its stack object) stays in frame storage, which the verifier
  compares;
- every Expression node sets the scratch pointer.

A tree whose source assigns `stack_obj` (`FrameFacts::keeps_exit_stack_obj`) keeps every
`stack_obj` store that only the exit makes dead.

The compiler's own bookkeeping restores still drop when dead. Keeping them too would change
open-pack trees whose early returns pass a restore nothing reads. So the rule follows the source:
an author who restores the cursor gets the restore.

### 6. A constant `global[NAME]` that is not 34 reads no global 34 (`mir/frame.rs`)

`may_read_global_34` resolves an index or scope token to a number when it is one of these:

- a literal;
- a raw block's source constant;
- a body `const NAME = N;`.

The name must name one number everywhere, and no `let` / `local` may reuse it. Then
`global[5]` (or `global[SLOT]` with `SLOT = 5`), or a `scope(S, ..)` with a known `S` other than
6, is not a global-34 read, and the tree is not held to main's rules for it.

## Consequences

- **What open-pack covers.** It builds byte-identical, so no shipped tree changed. The new
  behaviour shows only where a source uses the new forms or the item-5/item-6 shapes.
- **`self` after a search.** A conditional `find_best_object` keeps `self` the tree's own
  object (item 2), so the `stack_obj_clobber_after_self` lint (Track 7) correctly leaves it
  alone.
- **Out of scope.** Two pilot asks remain:
  - `remove_object` mode 2 (object 0, which no object is);
  - Maxis's pad bytes on Remove Object.

  Both are behaviourally equivalent, which is the bar.
