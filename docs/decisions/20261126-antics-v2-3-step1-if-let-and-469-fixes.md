# `.antics` v2.3 step 1: `if let Some(v) = f()`, flag methods on loop handles, three #469 fixes

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/v2/{ast,parser,specialize}.rs`
  - `src/v2/lower/*`
  - minimal arms only in `src/v2/lint.rs`
- **Track:** Track 8 (compiler), `.antics` v2.3 ergonomics, step 1
- **Builds on:**
  - [`20261125-antics-v2-2-temps-and-flags.md`](20261125-antics-v2-2-temps-and-flags.md) (the `Option<T>` return ABI)
  - [`20261125-antics-v2-2-step2-bitflags-and-retire-return-error.md`](20261125-antics-v2-2-step2-bitflags-and-retire-return-error.md)
  - [`20261125-antics-v2-pr-i4-and-std-utils.md`](20261125-antics-v2-pr-i4-and-std-utils.md) (#469)

## Decision 1: `if let Some(v) = f() { .. } else { .. }`

- **What it accepts.** `f` is a call to a `#[ret(temp = N)] -> Option<T>` tree.
  - An optional annotation `Some(v: T)` is checked as in `let Some(..) = .. else`.
  - An `else if` / `else if let` chain is one nested statement, as for `if`.
- **Lowering.** `if call { let v = temp[N]; then } else { else }`.
  - These are the nodes and locals of `let Some(v) = f() else { .. }`, and of the
    hand-written `if f() { .. temp[N] .. }`. Tests pin both comparisons byte for byte.
- **Scope.** `v` is bound for the then block only. It is unknown in the else block and after
  the statement, and an outer `v` is visible again after the then block.
  - Walkers that ask whether a name is rebound or written treat the binding as a rebinding.
    That is the conservative reading that `let` and `let Some` already get, so an outer
    `let v = 5;` stays a local instead of folding.
- **Refused scrutinees**, each with an error that names the fix:
  - any other expression, including a `bool` tree;
  - `run_functional`, `find_best_object` and `create_object`, which bind through the stack
    object, not a temp. They keep `let Some(..) = .. else { .. }`;
  - patterns other than `Some(name)`.
- **Formatting.** The formatter is token-based, so it needed no change. A test pins the
  layout and idempotence of an `else if let` chain.
- **Lint.** `lint.rs` only gains the arms Rust exhaustiveness needs, plus the init and the
  two blocks in its statement walkers, so findings inside the blocks are still reported.
  Lint has no rule that suggests `if let`. The lifter does not write it. Both are left to
  Tracks 7 and 9.

## Decision 2: flag methods on object-loop handles take the direct cursor lowering

- `for o in lot.objects() { o.flags2.set(F); }` used to leave the direct cursor lowering
  (`cursor_stmt_ok`), and saved and restored the stack object around the write. The
  operator form `o.flags2 flag_set F` did not.
- `cursor_stmt_ok` / `cursor_expr_ok` now accept the methods on a place that passes
  `cursor_place_ok`, or on a plain local. The methods are `set`, `clear`, `set_all`,
  `clear_all`, `contains`, `intersects`, `contains_all` and `contains_any`.
- Each method builds the operator form's bytes.
- Lint still suggests the methods only on `self`, a Sim and `stack_obj`. The #482 test that
  pinned the gap now says so.

## Decision 3: `std::flags::Expansions` bits 1, 2 and 5 stay unnamed

- `std/flags.antics` names a bit only if the engine reads or writes it.
- In the reimplementation, bits 1, 2 and 5 are set only by the install mount code
  (`crates/sims-content/src/install.rs`), which maps them to the installed pack's
  directory. Nothing in `sims-vm` tests them.
- Naming them would also mean naming products. They stay numbered (`flag_test 1`) until a
  reader is ported.

## Decision 4: three #469 lowering fixes

- **(a) A constant leaf of a returned `&&` / `||` sets the result.**
  - Case: `return temp[0] == 1 && false;`. When the test passed, the tree returned `false`
    with the last result left `true` by the test node.
  - A constant (or folded) leaf whose edge leaves the tree with a result now emits a
    `1 == 0` / `1 == 1` test node first. It is skipped when the previous node already left
    that result.
  - This holds for inline helper returns (`emit_inline_return_edges`) and tree returns
    (`emit_return_bool_expr`). Statement `if`s, whose arms do not exit with the result, are
    unchanged.
- **(b) A functional may not call a Sim tree.**
  - `run_functional` and Find Best Object run a functional's action and guard on whichever
    object asks, so the runner may be an object. `prepare_callable_call` now refuses
    `SimOnObject` calls from a functional (`object_reachable && !inherit_tree`).
  - Before, only Sim-only inherit trees were caught, through the call graph.
  - open-pack had no such call.
- **(c) A raw `if` whose arms end with the same instruction keeps its layout.**
  - Layout-free raw blocks (#469) are optimized like lowered text, so `merge_duplicate_tails`
    moved the shared last instruction after the `if`.
  - `raw_block_is_layout_free` now returns false when `mir::parse_body` finds, at any depth,
    an `if` with an else whose arms may end alike. Those blocks keep the pre-#469 layout.
  - Other passes run before the tail merge. One unwraps a single-pass `while true { ..; break; }`,
    so an arm may come to end like the other later. Arms are therefore known to stay apart
    only when every arm ends with an instruction and no two of those are equal.

## Decision 5: review round 1

- **`if let` needs a pattern after `let`.** The statement is an `if let` only when an
  identifier or `(` follows `let`. A variable named `let` (`if let { .. }`) stays a condition,
  as before.
- **Left as is:**
  - A binding named `self` (from `let`, `let Some`, `if let`, `for` or a parameter) is read
    by a bare `self`, but `self.x` still names the running object's own cell
    (`is_self_ident`).
    - This predates v2.3, and tests pin both a `for self` loop and a `self: Sim` parameter.
    - `if let Some(self)` behaves exactly like `let Some(self)`.
    - Settling it is a language decision, out of this step's scope.
  - Under `!`, a constant leaf's node leaves the leaf's own value, as a non-constant leaf's
    test node does. That is the opposite of what the tree returns.
  - A functional's call to a Sim tree is refused in code that constant folding discards too.
    This is the rule every context check already follows: discarded code is still checked.
  - After a test that restores the stack object (`run_functional`), the result is not tracked,
    so a constant leaf there always gets its node. It is correct, only one node larger.
  - A `const` in an else block cannot read an object `const K` when the then block binds a
    local `K`. This predates v2.3: a plain `let K` in the then block does the same.

## Consequences

- open-pack builds byte-identical to `main`.
- New tests: `tests/integration/v2_3_if_let_and_fixes.rs`.
- **Follow-ups:**
  - Track 7: lint binding walkers for `if let` (unused / shadowed binding); method
    suggestions on loop handles.
  - Track 9: lift `if f() { .. temp[N] .. }` to `if let`.
