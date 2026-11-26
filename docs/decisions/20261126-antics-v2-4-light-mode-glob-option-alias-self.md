# `.antics` v2.4: `light_mode`, the `GLOB` link, option bindings in place, `self` never bound

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/v2/{ast,parser,loader}.rs`
  - `src/v2/lower/{callable,env,mod,object,stmt,tree}.rs`
  - `src/v2/mir/{mod,pipeline,tests}.rs`, new `src/v2/mir/option_alias.rs`
  - `src/v2/std/globals.antics`
  - `src/{compile,manifest}.rs`
  - new `tests/integration/v2_4_compiler_upgrades.rs`
  - three lint test cases removed (`self` bindings), with no edits to `src/v2/lint.rs` or
    `src/v2/decompile.rs`
- **Track:** Track 8 (compiler), `.antics` v2.4
- **Builds on:**
  - [`20261126-antics-v2-3-step2-node-order-and-shared-tails.md`](20261126-antics-v2-3-step2-node-order-and-shared-tails.md)
    (#491: `narrow_callees`, `light`);
  - [`20261126-antics-v2-3-step1-if-let-and-469-fixes.md`](20261126-antics-v2-3-step1-if-let-and-469-fixes.md)
    (#489: `if let Some(v) = f()`);
  - [`20261125-antics-v2-typed-std.md`](20261125-antics-v2-typed-std.md)
    (#468: `std::globals`).

## Decision 1: object data word 16 is `light_mode`

`self.light_mode` (and `h.light_mode` on any handle) is object data word 16, an `i16`. It
compiles to the same bytes as `data[16]`.

What the word does:

- Light sources set it to 1 in their init and never change it. Switching a lamp off sets
  `light` (word 51) to 0 and leaves word 16 alone.
- The engine compares it with 1. The room count (0x467e05) then marks the light on the tiles
  around the object, its floor pools, and on the people in its room.
- The engine passes it to its light-map services as their mode.
- Removing the object clears the marks when the word is 1 (0x4b50b5).
- The legacy code calls it `light_mode` (`lot_environment/objects.rs`, `lot_placement.rs`).

Notes for other tracks:

- The doc comment of sims-render `lot.rs` `gives_light` says "on (word 16 is 1)". It is the
  light mode; on/off is word 51.
- `sims-people/tests/integration/lot.rs` models a lamp that is off by setting `data[16]` to
  0, which the original never does.

## Decision 2: `semi_global: "Name";` writes the `GLOB` link

An object (or template) meta field `semi_global: "Name";` names the semi-global library the
object links. The compiler writes one `GLOB` chunk:

- **Payload:** the name, NUL-terminated.
- **Chunk:** ID 128, flags `IFF_CONVERTED_FLAG`, empty label.

An install survey of 213 `GLOB` chunks found this:

- All of them use ID 128.
- 194 are length-prefixed, with a 256-byte payload and flags 0.
- 19 are NUL-terminated, with flags 0x10.
- The reader (`sims_formats::execution::global_link`) tells the two apart by the first byte:
  below 48 is a length.

The checks, which make every accepted name read back unchanged:

- 1 to 47 ASCII letters, digits, `_`, `-` or spaces;
- starting with a letter or digit, so the first byte is at least 48;
- no `.` (the engine appends `.iff` itself).

The formatter reflows tokens, so the new field formats like any other; a test checks that
it is idempotent.

## Decision 3: `extern "Lib" mod` tags semi-global declarations

A semi-global ID (`8192..`) names a tree of whichever library the running object links. A
declaration of one says nothing until it is tied to a library. That is why #491 never listed
a semi-global callee as narrow.

`extern "Lib" mod name { .. }` declares trees of library `Lib`. Its rules:

- **Only semi-global IDs.** A global ID is the same tree for every object, so it is refused
  inside a library block. `extern "Lib" bcon` is refused too.
- **Calls need a matching link.** An object that links another library, or none, cannot call
  through the block, since its IDs would name another tree. The engine matches names
  ignoring ASCII case, and so does this check.
- **Merging needs one library.** Two blocks with one name (a file's and an object's) merge
  only when both name the same library, or both name none.
- **Narrowness.** `frame_is_narrow(id)` for a semi-global ID:
  - is false for an object with no link;
  - needs at least one declaration in a block of the object's library;
  - takes every declaration of the ID in such a block, or in a block with no library, which
    might describe the same tree; all of them must be at most four arguments;
  - skips declarations in another library's blocks, which describe other trees.

  A raw block's inline calls benefit the same way (`raw_inline_callees`).

## Decision 4: `std::globals` frames count for every object, and 334 and 425 join it

`frame_is_narrow` now also reads `std::globals` (parsed once) for a global ID, whether or
not the file imports it.

- This matters for raw blocks. A raw inline `call 280(1, 0, 0, 0)` in a file without the
  import had no declaration at all, so `share_tails` or `node_order` refused the tree.
- A source declaration is already counted, so the std frames add evidence only where there
  was none. A wider declaration elsewhere still refuses the tree.
- A new header note says every tree declared in `std::globals` has a frame of at most four
  arguments.
- Each of the 64 declarations was checked against the global file's tree headers. Five read
  fewer parameters than their frame holds (278, 279, 340, 347, 417), all still at most four.

Two trees join `std::globals`, both checked against the original's headers and code:

- **334 `charge_off_home_lot(amount, check_only) -> bool`** (frame 2, no locals):
  - true at once on a residential lot;
  - otherwise it may charge `amount` through `alter_budget`, which tests first (flags 1) and
    charges after (flags 0), doubled on the date paths;
  - `check_only` 1 stops after the test;
  - false means the budget cannot cover the charge. That is an answer callers branch on,
    so `-> bool`.
- **425 `run_named_trees_until(amount) -> Result`** (frame 1, 4 locals):
  - runs the trees that table 303 names at entries 34 and 36, around a notify wait;
  - restores person words 25, 27 and 28;
  - its only false exit is a failed idle (280), so `-> Result`.

Each has a mechanical comment. The names describe what the trees do, as the file's other
names do.

**Deferred: 382 `-> bool`.** The roadmap pairs it with renaming the names that sit close to
the original labels.

- The lift writes 106 calls of `globals::ensure_standing()` in 66 behavioral corpus files,
  all from this signature (it reads `globals.antics` line by line).
- A local `-> Result` declaration of 382 merges with std today (the `merge_382` test), and
  would not after the change.

It is one joint change with Track 9 (the lift's call spellings) and needs its own byte
sweep.

## Decision 5: an option binding reads its temp in place

These forms bind `v` to the temp the call's true exit left the value in:

- `if let Some(v) = f() { .. }`;
- `let Some(v) = f() else { .. };`, on a `#[ret(temp = N)] -> Option<T>` tree.

Lowering copies the temp into a local, `let v = temp[N];`. The optimizer's copy forwarding
only joins adjacent stores, so `v` kept a frame word. Track 9 could not write `if let` in
strict mode, and a fixed frame needed a local for it.

A MIR pass (`alias_option_temps`) now reads `temp[N]` in place of `v` and drops the copy.
It runs before the optimizer, only on the bindings lowering recorded, and all or nothing per
binding. Its conditions:

- **`v` is never written.** No assignment, `let` or Expression target names it.
- **Every read is a plain Expression.** The statement or condition that reads `v` runs no
  primitive or call, so `temp[N]` is a value operand just as `v` was.
- **Nothing on any path from the copy to a read may write `temp[N]`.**
  - A primitive or call may write it.
  - So may an Expression whose target is `temp[N]`, a temp with a computed index,
    `stack.temp[..]` (the stack object may be this object) or a `scope(S, ..)` cell, unless
    `S` is a literal scope that never names a temp. The scopes allowed are object attributes
    and data, globals, literals, parameters, the stack object and person data. They do not
    include temps by temp index (11).
  - `if` arms are scanned from the state after their conditions, and the state after the
    `if` is that of every arm together.
  - A loop that may write `temp[N]` may not read `v` at all, since a later turn would see
    the write.
- **The dropped copy is unobservable.**
  - `N` is 0 to 7, which every frame has, so the copy could not fault.
  - The next node does not read the copy's last result.
  - The pass does not run under main's rules (global 34 reads the scratch pointer, which an
    Expression writes).
  - It does not run when the tree's text may read frame words, since freeing a word moves
    the others, or when the layout is kept.
  - It does not run after an inline call whose narrow frame is not proven (`wide_call`),
    since such a call may read the nodes after it.
  - A binding is recorded only where a call could stage its arguments in place
    (`copy_staging` is off). Both changes drop nodes and frame words, so the alias follows
    the staging rules:
    - not in an indexed (kind 4) tree, which is entered at a numbered node;
    - not inside a `#[temp]` loop;
    - not in an object lowered again in the copying form;
    - not in an object whose text may name frame words (`$Local:N`). Another tree's dialog
      may show a word this tree's frame left behind. In-place staging has the same exposure
      today, which is a follow-up.
  - An applied alias counts as a staging in place, so `finish` lowers the object again
    without it where nodes or frame words are observable. One case is a bottom frame left
    with no word, which Create Object writes. Another is an object with a tree that keeps
    its layout, which may read a frame word a sibling tree left behind.
- **Fallback.** Where a read is not covered, `v` keeps its local. That is exactly the bytes
  of copying the temp into a named local by hand; the tests compare both for a call, a temp
  write, a write to `v`, a calling loop and a raw temp write.

open-pack's `let Some` forms (`run_functional`, `find_best_object`, `create_object`) are not
recorded, so its bytes are unchanged.

## Decision 6: `self` cannot be bound

`self` always names the running object. It is now refused as the name of any binding:

- a parameter, including a Sim parameter (`fn t(self: Sim)`) and a helper's;
- `let` and `let .. else`;
- `if let`;
- `for`;
- a guard binding or an event binding;
- a block `const` binding (`const self = if .. { 7 } else { 9 };`, and tuples);
- a tuning parameter (`param self: i16 = 3;`), which a `const` binding may yield;
- an object or file constant (`const self = 3;`), which was accepted but could never be
  referenced.

The error suggests `sim: Sim`. The check sits in the parser's binding-name check, where
`stack_obj` and `world` are already refused, and in the check for block `const` names.

Before this change, a binding named `self` was ambiguous:

- A bare `self` meant the binding.
- `self.x` meant the running object's field.
- For `fn t(self: Sim)`, `self.n` read the object's attribute, but `self.data[29]` read the
  Sim's data.

Track 7 lint tests had three cases with a `self` binding (in `v2_rust_syntax.rs`,
`v2_lint.rs` and `v2_2_step2_lint.rs`). They no longer parse and are removed, each with a
note. Lint code that checks for a `self` binding is now dead (Consequences).

## Decision 7: review round 1

Sol's round 1 found five bugs, all fixed with tests (`option_alias_observability`, and the
`self` cases):

- **The alias pass ran where its dropped node or word is observable.** This covered:
  - an indexed tree's entry nodes;
  - an argless `main`'s only bottom-frame word;
  - a frame word a sibling tree reads after another tree's call;
  - the nodes after an unproven inline call.

  Decision 5's staging rules and `wide_call` check now cover all four.
- **`const self = ..` still bound `self`.**

Also changed:

- The `share_tails` diagnostic for a semi-global call now says which case applies: another
  declaration contradicts the library's, or no declaration of the library exists.
- A test now checks that a wide declaration in another library does not veto a narrow one
  in the linked library.
- The scope-11 temp write it flagged was already fixed before the review.

## Decision 8: review round 2

Codex stopped round 2 with a content filter before it reported. Its last note named two
paths still to check, and both are now fixed with tests:

- a sibling tree's dialog text reading a frame word (Decision 5);
- `const self` at object or file scope (Decision 6).

A fresh Claude Opus reviewer then reviewed the whole change. It found three bugs, all fixed:

- **A folded-away branch recorded its option bindings.** `check_discarded` lowers a dead
  branch only to check it, then reuses its fresh names. A later hand-written
  `let k = temp[0];` with the same name lost its copy. Bindings are no longer recorded inside
  a discarded check (test `a_discarded_branch_leaves_no_alias`, which fails without the fix).
- **Whether the pass applied was guessed from the binding's name missing from the body.** A
  later pass can drop or rename a name the alias pass left alone, such as a `#[slot]` local or
  an unread copy. The pass now reports whether it changed the body (`FrameFacts::aliased`).
  Only that report starts the staging check.
- **`param self: i16 = 3;` was accepted but could never be referenced.** It is now refused
  like every other `self` binding.

## Consequences

- **Track 7 (lint):**
  - add `light_mode` to the object word and self-field tables, and finish the Open Floor
    Lamp's floor pools and light on Sims with `self.light_mode = 1;` in its init;
  - lint's checks for a Sim parameter named `self` are dead code now;
  - the three removed test cases.
- **Track 9 (lift):**
  - spell `data[16]` as `light_mode` (byte-gated like `light`);
  - write `semi_global:` from an object's `GLOB` and `extern "Lib" mod` for its semi-global
    callees, which lets `share_tails` and `node_order` keep trees calling them inline;
  - strict-mode `if let` / `let .. else` no longer cost a frame word where the reads are
    covered;
  - calls to 334 and 425 now get std names, so the corpus needs a rebaseline.
- **Render and people tracks:** the `gives_light` comment and the dark-lamp test
  (Decision 1).
- **Follow-ups:**
  - in-place staging should also stay off where another tree's text may name frame words
    (Decision 5);
  - 382 `-> bool` with the renames (Decision 4);
  - a library block in `std` once a semi-global library's frames are checked the same way.
