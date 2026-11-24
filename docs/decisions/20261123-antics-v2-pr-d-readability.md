# `.antics` v2 PR D: Readability, Zero-`raw`, and Proof-Friendly Lowering

**Date:** 2026-11-23  
**Branch:** `antics/v2-pr-d-readability`  
**Track:** Track 8 (`.antics` v2 Language Ergonomics & Extensions — PR D)

## Context

A behavioural-equivalence pilot rewrote a 51-tree commercial object in v2 (local only; nothing from it is in the repository). The pilot found three problems:

- **Raw blocks.** 71 `raw { ... }` sites and 42 one-primitive shim `fn`s remained.
- **Proof-unfriendly output.** 47 of 51 compiled trees were INCONCLUSIVE under `verify-equiv`, although none were wrong. The causes:
  - a dead `__self_obj` save (25 trees);
  - header argument counts (8);
  - spelling `[0;8]` vs `[0xFF;8]` for argless calls (4);
  - extra loop and wrapper nodes (10).
- **A wrong table.** The v2 `person_field_by_name` table disagreed with `.air`.

Constraints:
- Everything lowers onto **existing** `.air` syntax. Nothing changes in the `.air` parser, lowerer, or verifier, so Track 7's parallel `.air` work merges cleanly.
- Every test fixture is synthetic.

## Decisions

### 1. Bug fix: one person-field table
`v2::lower::person_field_by_name` had `cooking = 9, charisma = 10, mechanical = 11, creativity = 14, body = 15, logic = 16`. The `.air` parser, the lifter, and the UI all use **10, 11, 12, 15, 17, 18**. The v2 table only feeds alias analysis (`canonical_air_cell`), so the bug could let `my.person.cooking` and `my.person[10]` be treated as different cells. v2 now reuses `crate::parser::person_field_by_name`, so there is a single table. A unit test pins the fixed mapping.

### 2. Proof-friendly lowering
- **Dead `__self_obj` save.** The `stack_obj` clobber analysis previously assumed every `raw { }` clobbers. Now it parses the raw text with the `.air` parser (wrapped in a synthetic tree) and walks the statements:
  - **Clobbers:**
    - mentioning `__self_obj`;
    - `create_object`, `find_best_object`, `find_best_interaction`, `gosub_found_action`, `make_new_character`;
    - `set_to_next` without `dest:` or with `dest: stack_obj`;
    - any operand write to `stack_obj` (assignment, `random`, `get_direction_to`, relationship reads, and an `inc_lt`/`dec_gt` condition, which stores its stepped value);
    - any `op 0xNNNN` other than opcodes known to keep it (`0x04`, `0x05`, `0x07`, `0x0F`, `0x19`, `0x23`, `0x2C`, `0x31`) and private/semi-global/global BHAV calls (`>= 0x100`).
  - **Not clobbers:** BHAV calls. `stack_obj` is per frame (copied into the callee and never copied back), so a call cannot clobber it.
  - **Parse failure** counts as a clobber.

  The other v2 clobber sources keep their existing rules. A v2 assignment to `stack_obj`, and passing `&mut stack_obj` to a helper, also count as clobbers.
- **Argless calls.** A 0-parameter `extern mod` or `tree` call lowers to `call ID(temps)`, which is `[0xFF;8]`, the SimAntics "no arguments" spelling. This is also the *temps pass-through ABI*: a callee that reads the caller's temps directly is declared with no parameters and called as `open(sim)?` / `if !foo() { ... }`.
- **`@frame(args: N, locals: M, format: F, revision: R)`** on a `tree` pins the `.air` header. Rules:
  - Every key is optional.
  - `args` may not be below the tree's positional-parameter count.
  - `format` must be `0x8000`, `0x8001`, or `0x8002`.
  - With `0x8000` or `0x8001` the VM gives a frame at most four arguments, so more (pinned or positional) is an error. The `.air` compiler already enforces those formats' local limits.
  - `@frame` on anything other than a standalone `tree` is an error.
- **Zero-node loops.**
  - `loop { }`, a constant-true `while`, and the internal loop wrappers (the general `while` cond loop and the inlined-helper return wrapper) lower to `.air` `while true { }`. That form has no condition node.
- **Constant folding.** A condition built from compile-time literals is folded in `if`/`while`. A constant-true `while` becomes `while true`; a constant-false one emits nothing. The literals are:
  - integer literals;
  - enum constants;
  - `const` items;
  - constant helper bindings (`ConstEnum`, `ConstBool`, literal `ConstI16`).

  For example, `kind == Menu::Meal` inside an inlined `publish_menu(Menu::Meal)` folds away. The untaken branch (or the body of a constant-false loop) is still lowered into a discarded buffer, so it stays type- and symbol-checked. This now also applies to plain `bool` constants (`if true { } else { ... }`). Enum values fold only for `==`/`!=`; an ordering compare takes the checked path and its error. Object `i16` params stay runtime `bcon.x` reads.
- **Tail-return bool helpers.** Some `fn -> bool` helpers are used as a condition with a body of *straight-line statements followed by one `return <cond>;`*. The body may contain loops, but no other `return` and no top-level `defer`. Such a helper is inlined without the `__fn_ret`/`__ret`/`__cond` loop wrapper, and its condition becomes the caller's branch.
- **`init`/`main` ID allocation.** This needed no change. Explicitly pinned private tree IDs are reserved first, and `init`/`main` take the lowest free private IDs. A test pins `@id(4096) tree` with `init` = 4097 and `main` = 4098.

### 3. Language features (no `raw`)
- **`loop { ... }`** is an infinite loop, like Rust's. It supports `break`/`continue`/`return`. `while true` behaves the same. Return analysis treats one without a `break` as never falling through, unless it is `cancel_on_abort` (a cancel leaves it, so a following `defer` still runs).
- **Counted loops.** Bounds are any `i16` expressions; non-literal ones are snapshotted once into fresh locals (start first, then end), as in Rust. The VM's `inc_lt`/`dec_gt` step the cursor with i16 wrap-around and then compare, which gives two forms:
  - **One node per iteration** whenever the adjusted bounds stay inside i16:
    - `a..b` → `i := a - 1; while i inc_lt b { ... }`, always (a wrapped `-32768 - 1` steps back to `-32768`);
    - `a..=b` → the same with `b + 1`, for a literal `b < 32767`;
    - `(a..b).rev()` → `i := b; while i dec_gt a - 1 { ... }`, for literal bounds above `-32768`;
    - `(a..=b).rev()` → the same with `b + 1`, for literal `a > -32768` and `b < 32767`.
  - **Edge-safe form** for every other range (a run-time inclusive or reversed bound, or a literal at the edge): `if a < b { i := first; while true { ...; if i == last { break; } i ±= 1; } }` (`<=` for `..=`). It never steps past the last value, so `0..=32767` runs 32768 times and `(0..-32768).rev()` runs zero times. `continue` runs the same step-or-leave check first.
  - `i` is read-only: assigning it, borrowing it as `&mut` (including inside a nested range's bounds), or naming it as a builtin's written local (`random_into`'s target, `animate(local:)`, and conservatively the local-slot arguments of `set_headline`/`push_interaction`) is an error.
  - The parser takes any start expression: `self.count..3` and `(n + 1)..m` are ranges; only `(a..b).rev()` is the reversed form.
- **Range patterns** in `match` on `i16`: `lo..hi` and `lo..=hi`, with literal bounds (negatives allowed).
  - Overlapping arms and empty ranges are errors.
  - Range patterns are rejected on `dialog(...)` and enum matches.
  - A one-value or one-sided range is a single compare. A two-sided range is a nested `if x >= lo { if x <= hi { hit := 1; } }` followed by `if hit != 0`.
  - Exhaustiveness rules are unchanged.
- **`animate(clip:, table:, flags:, events:, local:)`.** All arguments are named and compile-time, except `local:`.
  - As a statement, it runs to completion: `while !animate(...) { }`.
  - As a condition (`if animate(...)`), it takes one step: true means the clip finished, false means an event fired.
  - `animate(...) |event| { ... }` runs the body once per event, also as `while !animate(...) { ... }`. `break`, `continue`, and `return` work in the body.
  - `event` is read-only (writing or borrowing it is an error) and reads `param[0]`, where the VM stores events by default.
  - With `local: x` (an `i16` local), the VM stores events in `x` instead (flag `0x20` is set), and `event` names that local. Use `local:` in a tree whose `param[0]` is a real parameter. Flag `0x20` without `local:` is an error, since the events would land in an unnamed local.
- **`const NAME = <int>;`** works at module level (with `pub` + `use`) and as an object/template member. A constant is an `i16` when it fits. Otherwise it is a 32-bit GUID, usable only where a GUID is expected; using it as a value is an error.
- **`x.is_type(GUID)`** works on an object handle (`self`, a Sim, `stack_obj`, or an `ObjectId`/typed object). It lowers to `test_object_type(guid, object: ...)` and is usable directly as a condition, negated, or as a `bool` value.
- **Lot iterators.**
  - `for person in lot.sims()` lowers to `set_to_next(person, dest: v)`.
  - `for obj in lot.objects()` lowers to `set_to_next(all, dest: v)`.
  - The existing `objects_by_guid`/`objects_of`/`self.parts()` iterators are unchanged, except that `objects_by_guid` also accepts a GUID `const`.
- **Primitive builtins**, usable as statements (both edges continue), conditions, or `bool` values:

  | v2 builtin | `.air` primitive |
  |---|---|
  | `create_object(GUID, ...)` | `create_object` (statement / `if create_object(...)`; `stack_obj` is put back to its pre-call value on both edges, so use `let Some(x) = create_object(...)` to keep the object) |
  | `push_interaction(menu:, object:, priority:, flags:, icon:)` | `push_interaction` |
  | `run_tree_by_name(table:, index:, mode:, global:)` | `run_tree_by_name` |
  | `go_to_relative(location:, facing:, options:)` | `go_to_relative` |
  | `manage_inventory(op:, kind:, flags:, guid:)` | `manage_inventory` |
  | `refresh(target: self \| sim \| stack_obj, mode:)` | `refresh` (`self` is `stack_obj` in a Sim-on-object tree and `me` in `init`/`main`; `sim` is `me`) |
  | `action_label(table:, index:)` | `action_string` |
  | `show_string(table:, index:)` | `show_string` |
  | `set_headline(kind:, index:, duration:, options:, stack_obj:, local:)` | `set_headline` |
  | `special_effect(duration:, kind:, size:, flags:, str_index:)` | `special_effect` |
  | `breakpoint()` / `breakpoint(x)` | `breakpoint` |
  | `obj.run_functional(n, change_icon: true)` | `run_functional_tree` (existing) |
  | `random_into(x, n)` | `random` (in conditions: `random_into(x, n)?`) |

  Argument rules:
  - Arguments are named, and each key is checked for range.
  - Values must be compile-time: literals, enum or `const` values, constant helper bindings, or BCON entries with a known value. A `const` that does not fit in i16 is a GUID and only fits a GUID argument.
  - Local slots are named by v2 `i16` variables, never by raw indices.
  - Positional, unknown, and duplicate keys are errors.
  - Flag bits that make the VM read an operand as a local slot index are handled explicitly. `push_interaction(icon: x)` sets flag `0x01`, and `0x01` without `icon:` is an error. `push_interaction` flag `0x02` (object from a local) and `special_effect` flag `0x20` (duration from a local) are rejected, because those operands are raw slot indices that v2 cannot name.
  - `random_into` writes a direct `i16` slot: a local, attribute, or field of this object. A global range is copied into a fresh local first, because the VM's Random Number writes the draw into a global range operand (the original's pointer aliasing). The existing `random(n)` gets the same copy.
- **Name precedence.** The new builtin names (and the op wrappers below) were never reserved. A user `fn`, `tree`, or `extern` with the same name keeps winning, in statements, conditions, and values, so existing code is unaffected. `animate(...);` then calls the user function, and the `|event|` handler form is an error.
- **Fields.**
  - `sim.hand` is `my.slot[0]` (an `ObjectId`); `sim.slot[i]` is `my.slot[i]`. Both are read-only: the VM's slot scope has no write path.
  - `sim.age` is `my.person[58]`; `sim.person[i]` is `my.person[i]`.
  - `self.flags` is `data[8]`, an untyped bitfield that accepts any `bitflags` value in `.contains/.set/.clear`. Typed bitflags stay strictly typed.
  - `advert.delta[m]`, `advert.personality[m]`, and `advert.minimum[m]` are the advertisement scopes `scope(12, m)`, `scope(28, m)`, and `scope(29, m)`. `m` is a motive (name, `Motive::X`, or `0..16`).
  - `stack_obj` can be read (`ObjectId`) and assigned. Assigning it counts as a clobber, so the self save is kept. Restore with `stack_obj = self`.
  - `let x: T;` declares a variable without initializing it. It reads whatever the local slot holds (0 at tree entry); there is no definite-assignment check.
- **Low-level opcodes as `op` builtins.**
  - `find_best_interaction()` (0x0003), `grab()` (0x0004), `drop()` (0x0005), `gosub_found_action()` (0x001E), and `notify_out_of_idle()` (0x0031) lower to `op 0xNNNN (0, ..., 0)`.
  - The raw `op` form keeps them independent of `.air` primitive naming.
  - They leave `stack_obj` exactly as the primitive does, so `gosub_found_action()` sees `find_best_interaction()`'s pick. The clobber analysis still counts the first two as clobbers, so the self save is kept.

## Consequences

- Most pilot raw sites and shims now have a v2 spelling. The remaining exits from inlined helpers are covered by `return error;` and tail-return bool helpers.
- The output is closer to what the original authoring tools produced: no dead save, `[0xFF;8]` argless calls, pinned headers, and fewer always-true and wrapper nodes. The aligner prototype from the pilot is not upstreamed; that stays future work.
- **Review hardening.** A first review round found the range edge cases, extra `stack_obj` writers, the cancel/`defer` path, and the builtin argument holes above. Each fix has a synthetic regression test; the range edges and folded loops run live in the VM.
- **Open-pack parity.** All 20 objects still compile, and the behavioural tests pass. The expanded `.air` changes in 8 objects, all intentionally:
  - `while 1 == 1` became `while true`: bookshelf, cot, dining chair, sofa, wall phone, shower, toilet.
  - The floor lamp's compile-time `bool` param branch (`if 1 != 0 { ... } else { ... }`) folds to the taken branch.
  - The tail-return bool helpers `repair_by_progress` (shower) and `run_timed_cycles` (toilet) inline without their wrapper loop, flag, and copy nodes. The work loop is unchanged, and the final compare becomes the interaction's branch.
