# `.antics` v2 PR G: Pilot Bugfixes, Missing Builtins, and Cheap Lowering Wins

*Decided 2026-11-24.*

**Branch:** `antics/v2-pr-g-pilot-fixes-and-features`  
**Track:** Track 8 (`.antics` v2, PR G)

## Context

Four multi-object behavioural-equivalence pilots and a fourth Fridges round rewrote commercial objects against PR F. They ran locally, and nothing from them is in the repository. Their reports list three kinds of gaps:

- **Bugs.** Silent mis-binding of builtin arguments, an entry save the source never needed, an inverted return polarity, and no way to write a BHAV with zero nodes.
- **Missing builtins and fields.** These forced `raw { }` blocks.
- **Lowering costs.** Locals and nodes the original frames do not have.

Constraints:

- **Same as PR F.** Everything lowers onto existing `.air`, and only `src/v2/{ast,parser,lower,opt}.rs` and the v2 tests change.
- **One new field.** `TreeFrame` gains `empty`, which needs no new match arms elsewhere.
- **Synthetic tests.** Every test is synthetic.

## Decisions

### 1. Correctness

- **V-1: builtin arguments bind strictly.**
  - One binder (`bind_builtin_args`) now serves the slot and route primitives, `set_motive_change`, `stop_all_sounds`, the budget calls, `find_best_object`, `create_object`, `generic_call`, `play_sound`, `job_data`, and `distance_to`.
  - Positional arguments fill the documented order.
  - The binder rejects four mistakes, each with an error naming the primitive:
    - an unknown name;
    - a duplicate;
    - a positional argument after a named one;
    - too many positional arguments.
  - Before this, `route_to(front, source: 0)` silently took `source` as the flags.
  - `route_to` gains `source:`, which takes a param, an object or slot, or a global (0–2).
  - `snap` gains `mode:`, which takes a param, an object or container, the front, a slot, or a global (0–4).
  - Both also accept a number or a `const`.
- **N2: the entry save is bookkeeping, not program.**
  - A Sim tree's `let __self_obj = stack_obj;` is dropped whenever nothing reads it. This holds even before a node that observes the last result: that node then sees the frame's entry state, as the source says.
  - **This reverses PR F's `next_ignores_last_result` guard** for the save only (`20261124-antics-v2-pr-f-zero-raw-and-sugar.md` §1, L7). Other dead stores keep that guard.
  - **Guard.** Dropping the save moves every later named local down one frame word, and a frame word keeps whatever an earlier frame left there. So every named local must be written before it is read:
    - When the structured pass parses the body, the flow analysis checks this.
    - When it cannot (a raw block with several statements on one line), the save is dropped only if no local is declared without a value (`let x;`).
- **Raw clobber analysis decodes operands like the VM.** It replaces an opcode allow-list.
  - These primitives write the caller's `stack_obj`, and so need the save:
    - Expression into scope 10;
    - Random and Get Direction To with a scope-10 destination;
    - Relationship with a scope-10 value;
    - Set To Next without a separate destination;
    - Find Best Interaction, Find Best Object, Make New Character, Gosub Found Action, and Create Object.
  - Calls and the other primitives do not.
- **L2.** `return <simple compare>` keeps its polarity.
- **G-S13: `@frame(empty)`.**
  - A tree with this frame emits zero nodes, which the original has for unused BHAVs. Its body must be `{}`, and it need not return a value.
  - A plain `{}` body still emits one `return true;` node.
- **Known semantics, unchanged.** A `raw { }` block never restores `stack_obj`. After a raw block that moves it, `self.n = 1` writes `stack.attr[0]` on the moved object. The tests now state this.

### 2. New builtins (Tier 2)

| Builtin | Opcode | Notes |
|---|---|---|
| `change_suit(index:, scope:, remove:)` | 6 | |
| `burn(mode:, flags:)` | 9 | |
| `look_towards(mode:)` | 22 | |
| `drop_onto(src:, dst:)` | 43 | |
| `find_location(mode:, flags:, local:)` | 16 | |
| `relationship(column:, dir:, flags:, local:, value:)` | 26 | `value:` is required and must be an `i16` local, which the optimizer treats as written. |
| `old_relationship(write:, column:, param:, dir:, flags:)` | 24 | |
| `generic_call(routine)` | 1 | |
| `play_sound(sound, target, flags:)` | 23 | Takes an FWAV id or `const` as well as a `sound` binding. `flags: 0x02` (stack object) is rejected in favour of `target`. |
| `stop_all_sounds(target)` | 48 | Its target is checked like `play_sound`'s: `self`, `sim`, `stack_obj`, or a literal bool. |
| `job_data(career, level, field)` | — (scope 33) | Stages `temp[0]` / `temp[1]`. A level read from `temp[0]` is copied first, and call arguments are lowered before any staging. |
| `distance_to(target, from:, temp:)` | 11 | Points `stack_obj` at the target around the primitive, so the save analysis counts it unless the target is `stack_obj`. A `from: stack_obj` is copied before the move. |

### 3. New fields (Tier 2)

- **Sim fields:**
  - `sim.attr[i]` and `sim.data[i]`, readable and writable.
  - `sim.container`, which is object data word 2 and read-only.
  - Writable `sim.priority`.
  - `sim.personality.{nice, active, playful, outgoing, neat}`. These are person words 2, 3, 5, 6, and 7, as the character-file writer and the structure lifter label them. Foreign Sims have them too.
- **Handle flags.** `handle.flags` (object data word 8, any bitflags) on every handle, including `stack_obj.flags`.
- **Already on main.** Mutable `param[n]` needed no change.

### 4. Lowering wins (Tier 3)

- **`while` conditions.** `while f()` and `while !f()` test a call condition directly at the top of the pass, with no `__wcond` local.
- **Identity stores.** The optimizer drops `x := x` for temps, params, `local[N]`, and locals when the next node ignores the last result. This removes the `temp[i] := temp[i]` that forwarding a staged argument copy leaves.
- **`local[N]` reads.** A read pins the layout like a raw block: `.air` puts named locals after every explicit slot, and the optimizer skips the tree. It cannot share a tree with a pinned `push_interaction` object, and writes are rejected.
- **Const patterns.** A `const` name in a `match` arm matches its value, with the usual duplicate and range checks.
- **Brace-free arms.** `0 => f(), 1 => x = 2, 2 => x += 1` parse. A full statement (`3 => return;`) still works.

### 5. Deferred, with reasons

- **In-place `stack_obj` stepping, `create_object` into `stack_obj`, and the `set_to_next` category / any modes.**
  - Today v2 restores `stack_obj` at statement granularity. Leaving it moved needs a flow-sensitive "stack object displaced" state, because every later `self.` access would have to restore or go through a foreign cell. Without that state, a moved `stack_obj` would repeat the raw-block hazard above.
  - That state touches every self access, so it gets its own PR.
  - Fridges 4112 now stops exactly here: Maxis steps `stack_obj` in place, and v2 emits `dest: stack_obj`.
- **Tier 3, needing AST or formatter changes Track 9 owns:**
  - labeled `break 'l` / `continue 'l`;
  - `@slot(N) let x;`;
  - `@frame` on `main` / `init`, and no phantom Init/Main stubs;
  - context polymorphism (Sim↔object helper calls, `me.person[i]`);
  - zero-cost inline helpers (no `__arg_` copies, `Guid` params, `fn -> Result`).

  Each needs `fmt` / `lift` support to round-trip, or changes the helper ABI, so each is a follow-up.
- **Zero-local `||` outside `if`.** Short-circuit lowering uses single-pass loops, so a `break` arm cannot reuse it in a `while` condition.

## Validation

- `cargo test -p sims-antics`, `cargo clippy -p sims-antics --all-targets -D warnings`, and `cheats_cache_and_open_pack` all pass. `open-pack` builds unchanged.
- The local Fridges pilot stays at 50 of 51 trees equivalent and 1 inconclusive (4112, above) under `--behavioral`, and the whole-file run is unchanged.

## What would reopen this

- **Entry-save policy.** A verifier result showing a tree whose behaviour depends on the dropped save's frame word or node.
- **Personality words.** A sim trace contradicting the person-word numbering for the traits. `sims-sim`'s motive decay names word 3 `NEAT`, against the character-file writer's 7, and Track 1 should check this.
