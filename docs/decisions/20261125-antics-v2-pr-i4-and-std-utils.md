# `.antics` v2: PR I-4 and the `std` utility modules

*Decided 2026-11-25.*

**Branch:** `t3/antics-v2-pr-i4-and-std-utils`  
**Track:** Track 8 (`.antics` v2)

## Context

The pilots and the corpus kept raw `.air` for several things v2 could not say:

- the temp-indexed and neighbor scopes (27, 30, 31, 24, 32);
- operand bytes past what a primitive's arguments spell (`play_sound`, `clear_motive_changes`);
- Make New Character, Reach, and a Drop Onto slot from a parameter;
- `#[context(inherit)]` trees calling Sim trees (the Music Recording Studio session).

Two other costs fell on every tree:

- inline helpers copied every non-temp argument into a hidden local;
- one raw block anywhere in a tree turned the backend optimizer off for the whole tree.

PRs #461 and #466 also left three lowering bugs open, each fixed in section 3.

Every test is synthetic and lives in `tests/integration/v2_pr_i4_and_std_utils.rs`. Its
`std_utils` module tests the `std` modules and its `pr_i4` module tests everything else. The
open-pack build is byte-identical to `main` (b4bbe414e), with `open-pack/common/helpers.antics`
delegating to `std::motives`.

## Decision

The code is the definition of each rule. This section says what changed and why.

### 1. Scopes (`lower/lvalue.rs`, `lower/expr.rs`, `lower/tree.rs`)

Each meaning was read from `sims-vm` `memory.rs`.

| Spelling | Scope | Notes |
|---|---|---|
| `h.sprite_flags[temp[k]]` | `scope(27, k)` | The index is the number in `temp[k]`, so no setup node is needed. Any other index is still set into `temp[0]` first, as before. |
| `sim.person[temp[k]]` | `scope(30, k)` | The running Sim's person word whose number is in `temp[k]`. Readable and writable. |
| `stack_obj.person[temp[k]]` | `scope(31, k)` | The stack object's person word, numbered the same way. |
| `h.neighbor.age`, `.person_on_lot`, `.in_selected_family`, `.friend_count`, `.family_house`, `.family_friend_count`, `.field[i]` (0..=9) | `scope(24, i)` | Read-only: the engine hands back a value, not a word. A write is an error, and so is an index past 9. |
| `h.neighbor.<person word>`, `h.neighbor.person[i]` | `scope(32, i)` | Read-only, like scope 24. |

- `fame` is person word 81 (half-stars), so `sim.fame` and `h.neighbor.fame` read it.
- Scope 12 (the check tree's advertisement) was already spelled `advert.delta[m]`, and 28/29 are
  its siblings. The corpus's 260 raw scope-12 sites come from the lifter not using that
  spelling, which is Track 9's gap, not the compiler's.

### 2. Primitive options and call ergonomics

**`padN` bytes** (`lower/actions.rs` `split_pad_args`): `play_sound(.., padN: 0xNN)` and
`clear_motive_changes(padN: 0xNN)` set operand byte N (0..=7). The `.air` is
`pad(N: 0xNN, ..)`, which `verify` checks: the bytes must be in ascending order and must change
something.

- A byte the call's own arguments spell is an error. For `play_sound` that is bytes 0, 1 and 4,
  which the VM reads.
- Naming the same byte twice is an error.

The corpus's volume and loop bytes are spelled as pads, not named options, because their
meaning is not verified in `sims-vm` (it does not read them). Naming them would claim a
semantics nothing checks.

**`reach(mode:, moves:, slot:)`** (opcode 47):

- Mode 1 reads the slot number from a parameter, so `slot:` must name a tree parameter (a
  parameter variable or `param[N]`), and the index goes in the operand.
- Mode 1 without `slot:` is an error, and so is `slot:` with any other mode.

**`make_new_character(skin:, age:, sex:, species:)`** (opcode 19):

- skin, age and sex must be `i16` locals (the VM reads locals at operand bytes 0–2). An `enum`
  local is refused: the VM reads the code whole, with no range.
- species is a constant 0..=2: person, dog or cat. Other species are Unsupported in the VM.

**`drop_onto(src:, dst:)`** accepts a constant or a tree parameter, spelled `param[N]`.

**Inherit trees calling Sim trees** (`lower/callable.rs`, `lower/mod.rs`
`InheritCallFacts`):

- A `#[context(inherit)]` tree may call a Sim tree by name with `me` (its runner) as the Sim:
  `open_door(me, ev)?`. The call is the same `call N(temps)` in both passes of the inherit
  check, so the check's byte comparison still holds.
- The callee runs on the inherit tree's runner and current stack object. That is the Sim's
  context only when the inherit tree itself runs on a Sim. So an inherit tree that names a Sim
  tree, and any inherit tree that calls one of those (computed to a fixpoint), is refused when
  called from an object's own tree (init, main, other lifecycle hooks, `fn` trees without a
  Sim).
- A functional's action and guard count as object callers too: `run_functional` and Find Best
  Object run them on whichever object asks.
- Calls from interactions, Sim trees and other inherit trees are allowed.
- Code a literal condition discards emits no call, so it adds no fact.
- A Sim tree reached this way follows PR D's rule: its `self` is the stack object, which may be
  wherever the inherit tree moved the cursor. For example, the studio's session trees run with
  the cursor on the console.
- A raw `call N` by number cannot be checked.
- On the Music Recording Studio round-4 source, replacing the `open_door` raw block with the
  call gives the same `.air` (only the raw block's line breaks differ).

**Read-only helper parameters without a copy** (`lower/inline.rs`
`can_forward_helper_cell`, `HelperCell::Pure`). A read-only inline helper parameter now reads
any plain cell in place when both of these hold:

- **The cell's read cannot fault or wake anything.** `safe_cell_air` decides this, with bounds
  from `sims-vm` `memory.rs`. Reads that qualify:
  - `temp[0..8]`, a parameter below the tree's argument count, `global[0..32]` (older saves
    keep 32);
  - a declared `my` attribute, and `my` data words below 64 (the smallest object), except
    word 25, whose read wakes the object;
  - the runner's person words below 80 and motives below 16, only in a Sim tree, where the
    runner is a Sim. Inherit trees and functionals (action and guard) do not qualify: an
    object may run them.

  A read through the cursor (`self` in a Sim tree) is never forwarded: nothing proves the
  entry cursor names a live object of this kind.

  Because the read cannot fault, moving it into the body, repeating it, or dropping it is
  unobservable.
- **The helper is pure.** It writes nothing but its own locals and calls nothing, so neither
  the cell nor the cursor changes while it runs.

Temps, parameters and own attributes keep their existing rules. A stack-object or foreign read
is still copied up front. `max(sim.age, global[3])` from `std::math` now costs no local.

**Scoped `keep_layout`** (`lower/raw.rs` `raw_block_is_layout_free`, `layout_free_raw_lines`).
A raw block no longer pins the tree's layout when it is built only from text the lowering itself
emits:

- `call N(temps)` calls;
- the primitives `find_best_object`, `run_functional_tree` and `play_sound`;
- control flow;
- temp, parameter, global, own, Sim and stack-object cells.

It must contain no `local[N]`, named local, declaration, `const`, string, label or `->` graph,
`scope(..)`, `expr(..)`, `pad`, block comment or `#` comment (the line printer strips only `//`). The invariants that make this sound:

1. **No frame word is named.** There is no local, so coalescing, renaming and pinning cannot
   change what the block reads.
2. **No instruction reads bytes past its own operands.** Argless calls pass the temps, and the
   three primitives were checked in `sims-vm` to read only their 8 bytes and none of the
   caller's frame words. So the block does not depend on its neighbours' layout.
3. **Its text is lowered text.** The block is printed one statement per line, as the lowering
   prints its own code, so the backend passes parse it and see exactly what structured code
   would give them. The test compares each such block with its structured spelling and gets
   identical trees.

Two more rules keep the optimizer from changing what such a block does:

- A block that may write the cursor marks the tree like a source `stack_obj = x`, so its cursor
  stores are not dropped as dead at the exit, whose result they set.
- An inlined helper's arms that hold raw code keep the single-copy branch lowering, so the raw
  code is not copied to each return site (a tree's author counted one copy against the node
  limit).

Every other raw block keeps the old behaviour: the whole tree's layout is kept and only the
unread entry save may go. The tests that pinned that path now use a raw `const` to keep it.

### 3. Lowering fixes

**Bug 1: a constant inline tail recorded the previous call's result** (`lower/cond.rs`
`emit_inline_return_edges`).

- When an inlined helper's tail is a compile-time constant, its arm used to jump straight to the
  exit. The last result was then whatever the call before it left.
- Now, when the taken arm exits the tree with a result (`return true;` / `return false;`), the
  lowering emits a constant test node first: `if 1 == 1 { .. }` or `if 1 == 0 { }`.
- The test is skipped when the previous node already left that value: a cell assignment leaves
  true, and an `if <test> {` leaves its polarity on entry to the arm.
- Arms that do not exit with a result, and error exits, are unchanged.

**Bug 2: a foreign read restored the entry cursor** (`lower/stack.rs`, `lower/ast_facts.rs`
`ExplicitStackOps::for_tree`). In a tree that is not explicit, `stack_obj = o; x = p.attr[3]`
put back the entry object after the read, not `o`.

- A new `reassigns_stack_obj` fact marks any tree that assigns `stack_obj` (or borrows it
  `&mut`).
- In such a tree, foreign reads, `emit_on_self`, distance, `run_functional` and lvalue moves
  save and restore the current cursor (`__pre_stack_obj_N`).
- A raw block that may write the cursor has the same effect from that point in lowering order.
  A loop (`while`, `for`, `repeat`, an animation's event loop) whose body holds one sets it
  before the body, since the next iteration runs the code before the block again. So does an
  inlined condition helper holding one, whose arms are captured before the helper is lowered,
  and a `run_functional` argument holding one counts as moving the cursor.
- Make New Character counts as a cursor move: on success it leaves the new neighbor in the
  stack object. A raw block after a loop does not change the loop (PR H's `for ..
  else` shape is kept).
- PR D's `self` rule is unchanged, and so is the explicit-tree classification.
- `for` loops still restore the entry object on exit (PR H's rule, pinned by its tests).
  Treating reassigning trees as explicit there changed both, so it was not done.

**Bug 3: a partial write before a faulting read** (`lower/expr.rs`
`can_lower_arith_in_place`, `expr_may_fault_read`).

- `temp[2] = 1 + stack_obj.object_type` used to lower in place: `temp[2] := 1;` then
  `temp[2] += scope(21, 9);`. If the stack object is gone, the read faults after `temp[2]` has
  been written.
- In-place arithmetic into any cell other than a fresh local now goes through `__arith` when an
  operand after the first, or a negated base, may fault.
- What may fault: any read that is not one of the safe cells listed under read-only helper
  parameters (`safe_cell_air`). That covers reads through the stack object, a foreign handle, a
  neighbor or a definition, words past the smallest owner's count, and `self` read through a
  cursor the tree may move, even when the destination goes through the same cursor (a word past
  the object's count faults after the first write lands).
- A temp-indexed destination (`scope(27|30|31, k)`) is always staged: its index may alias a
  cell the right side reads.
- A `&mut` to a temp-indexed cell copies the index at the call and sets it into `temp[0]`
  before each use (as a run-time sprite index already was), so a helper that rewrites the
  index temp still writes the borrowed cell.
- The in-place shape `temp[4] := 1; temp[4] += stack.attr[0];` is still emitted for a `self`
  read in a Sim tree that never moves the cursor. It relies on PR D's contract that a Sim
  tree is entered with the stack object on an object of this kind. That shape predates this
  PR and is open-pack's, and a caller that breaks the contract already reads the wrong
  attributes.
- A Sim tree that never moves the cursor keeps `temp[4] := 1; temp[4] += stack.attr[0];`, which
  is the open-pack shape, so the pack is unchanged.

**Temp staging reads.** The sprite-flags setup and Job Data's career both write `temp[0]`
first. An operand read through a cell indexed by `temp[0]` (`scope(27|30|31, 0)`), not only
`temp[0]` itself or job data, is now copied before (`lower/mod.rs` `reads_temp0`).

### 4. `std::math`, `std::motives`, `std::budget` (Stream 1b)

- `std::math`: `min`, `max`, `clamp`, `abs`, `chance_percent`.
  - `abs(-32768)` is -32768, because the expression primitive keeps the low 16 bits.
  - `clamp` with `lo > hi` does not panic: an `x` below `lo` gives `lo`, and any other `x`
    gives `hi`.
- `std::motives`: the five cancel-safe restore and repair loops, with the same bodies as
  open-pack's helpers. `open-pack/common/helpers.antics` now delegates to them
  (`use std::motives::X as std_X;`), and the pack is byte-identical.
- `std::budget`: `try_charge(sim, price, category)` is one Alter Budget node, since the
  primitive tests the funds before it spends.
  - `category` is a `BudgetCategory`.
  - An `i16` goes through `BudgetCategory::from_raw(n)`, which now binds a helper's enum
    parameter as a constant (`lower/consteval.rs`).
- The loader registers the three modules, and the unknown-module error lists all six.
- `std::globals` tree 382 stays `-> Result`, and no `std::globals` name changed.

## Consequences

- The neighbor, temp-indexed and fame cells, the pads, Reach, Make New Character, Drop Onto
  parameters and inherit→Sim calls need no raw block.
- Trees with a call-only raw block, or another lowering-shaped one, are optimized like the rest.
- Follow-ups for other tracks:
  - **Track 7 (lint):** `lint.rs` `file_callables` assumes a `std` glob import brings in no
    callables. The new modules export helpers, so `use std::math::*;` callers may need it.
  - **Track 9 (lift):**
    - scope 12 could lift to `advert.delta[m]`;
    - the new scopes, pads and primitives are new lift targets;
    - the lift corpus effect of `from_raw` constant folding is unchecked.
- Not done:
  - checking a raw `call N` to a Sim tree from an inherit tree;
  - a functional calling a Sim tree (directly, or through one that reaches a Sim-only inherit
    tree) when an object runs the functional. The language types a functional's runner as its
    `sim` binding, and Sim trees called from functionals predate this PR. Closing the gap
    needs a functional-runner type rather than one more inherit fact;
  - a short-circuit condition with a constant leaf (`temp[0] == 1 && false`) can still leave the
    previous node's result as the tree's last result. That predates this PR and is left for a
    follow-up;
  - named `play_sound` volume and loop options, which wait on a VM reading of those bytes.
