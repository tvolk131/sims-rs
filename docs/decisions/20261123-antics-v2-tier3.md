# `.antics` v2 Tier 3: First-Class Dialog Matching, Enums/Bitflags, Typed Cross-Object Refs, Autonomy Guards, Lifecycle Hooks, and Compound Expressions

**Date:** 2026-11-23  
**Branch:** `antics/v2-tier3-dialogs-enums-and-expressions`  
**Track:** Track 8 (`.antics` v2 Language Ergonomics & Extensions — PR B)

## Context

Following PR A (#395, Tiers 1–2 ergonomics, which brought `open-pack/` down from `1,709 -> 1,410` lines), six remaining low-level seams still surfaced in `open-pack/` and real SimAntics authoring:
1. Manual `STR# 301` tables and raw `temp[0] == 0` branching for 3-button dialogs (`open-wall-phone.antics`).
2. Untyped magic integers and raw 1-based bit numbers (`flag_set 1`, `flag_clear 2`) for state machines and occupancy masks (`open-wall-phone.antics`, `open-double-bed.antics`).
3. Raw `self.data[4] = 1; self.data[42] = 3;` placement words (`open-meal-plate.antics`) and raw `attr[0]` / `temp[2]` / `temp[3]` / duplicate magic GUIDs across cross-object collaborations (`open-wall-phone.antics`, `open-fridge.antics`, `open-counter.antics`, `open-stove.antics`).
4. No typed way for interaction guards to distinguish autonomous Sim probes (`param[0] == 1`) from player pie-menu checks (`param[0] == 0`).
5. No high-level syntax for OBJf lifecycle hooks `load` (slot 2), `reset` (slot 3), and `cleanup` (slot 4).
6. Compound arithmetic/bitwise expressions spilling intermediate scratch locals instead of collapsing left-associative chains in-place when alias- and fault-safe.

All six features lower 100% cleanly onto the existing `.air` IR without modifying `.air`, `sims-vm`, `sims-ui`, or `sims-game`.

## Design & Lowering Summary

### 1. First-Class `match dialog(...)` + Auto-Interned Inline Strings (`STR# 301`)
- Inline string literals in `dialog(...)` (`title`, `message`, `button1`..`button3`, `buttons: ("...", ...)`) auto-intern into `STR# 301` via `DialogStringTable`, preserving any explicit `strings 301` entries in-order, deduplicating identical strings, and enforcing the 255-entry SimAntics limit.
- `match dialog(...)` supports 2-way (`yes_no`) and 3-way (`yes_no_cancel`) dialogs with ergonomic arms (`Button1`/`Button2`/`Button3`, `Yes`/`No`/`Cancel`, `0`/`1`/`2`, `_`), compile-time exhaustiveness, arity validation against effective dialog kind (`0 | 3 | 4` -> 1, `1` -> 2, `2` -> 3), unified `flags`/`inform`/`icon_mode`/`temp` normalization (rejecting raw `flags & 1` inform bit), and automatic save/restore of implicit `temp[0]` on 3-way branches before executing user arms or `defer` cleanups.

### 2. Zero-Cost `enum` and `bitflags` Types
- Module-level `pub enum` / `pub bitflags` (importable via `use`) and object/template-local declarations lower to zero-cost SimAntics `i16` values with strict nominal type checking across `state`, locals, helper parameters/returns, and comparisons.
- `enum`: auto-incrementing `i16` discriminants (checked against overflow and duplicate values), mandatory explicit initializer on `state` when `0` is not a declared variant, and type-aware exhaustiveness checking in `match` (shared identically by lowering and all-paths-return / divergence analysis).
- `bitflags`: 1-based SimAntics bit positions (`1..=16`, including bit 16 sign bit `-32768`), duplicate bit rejection, reserved `None = 0`, single-bit constant methods `.set()`/`.clear()`/`.contains()` lowering directly to single-instruction `flag_set`/`flag_clear`/`flag_test` (with `&mut` auto-deref on parameters), and multi-bit/runtime masks lowering to exact bitwise ops (`|=`, `&=`, `^=`).
- Built-in type names (`i16`, `bool`, `Object`, `ObjectId`, `Sim`, `Motive`, `Skill`, `Sound`, `Slot`, `Height`, `Placement`, `Self`) are reserved across declarations, generated object type names, and `use ... as` aliases; `ObjectEnv::lookup_named_type` provides a single consistent lexical resolution order (`local -> built-in -> module`).

### 3. Named Placement Properties & Typed Cross-Object References
- `self.allowed_heights` (`data[4]`) uses built-in `Height` bitflags (`Floor`, `LowTable`, `Table`, `Counter`, `NonStandard`, `InHand`, `Sitting`, `EndTable`); `self.placement` (`data[42]`) uses built-in `Placement` bitflags (`Floor`, `Terrain`, `Water`, `Sloped`, `Air`, `Pool`, `WaterOnly`), matching `sims-vm/src/buy.rs` and `placement/find.rs`.
- Objects export a PascalCase schema type (`OpenFrostFridge`, `OpenOakCounter`, `OpenHearthStove`, `OpenMealPlate`, etc.) resolved across modules (with kebab-case filename fallback `open_fridge -> open-fridge.antics`), carrying `master_guid`, `lead_guid` (matching `compile.rs`'s explicit `lead` / `(0, 0)` origin selection), `tile_guids`, typed `state` slots, and typed `functional kind(...) -> RetType` signatures.
- Typed cross-object operations:
  - `for x in objects_of::<T>()` iterates `set_to_next(guid, T::lead_guid)` yielding typed `T` handles (and `self.parts()` yields typed `Self` handles).
  - `let Some(x) = create_object::<T>(...) else { ... }` spawns `T::master_guid` and rejects `FROM_RECORD` (`flags & 0x04`) which would override `T`'s GUID at runtime.
  - `let Some(x) = find_best_object::<T>(kind [, flags: f]) else { ... }` runs the native `find_best_object(kind)` fast path, verifies runtime GUID membership against `T`'s tile GUIDs, and falls back to a typed `set_to_next(guid, T::lead_guid)` scan (filtering `in_use == 0`, `!DISABLED`, optional room match, inline-verified simple state guard, and positive score) so a higher-scoring foreign object can neither cause type confusion nor starve a valid lower-scoring `T` (while never bypassing complex guards). *Future VM enhancement:* native GUID-filtered `find_best_object` in `sims-vm` can preserve distance-attenuated ranking under foreign competition without changing `.antics` v2 syntax.
  - Typed `functional kind(...) -> RetType` returns its value in `temp[0]` and binds via `let Some(v) = obj.run_functional(kind, ...) else { ... };`. Untyped `ObjectId.run_functional` forbids manufacturing nominal types (`ObjectRef`, `Enum`, `Bitflags`, `Bool`).
- **Known limitation (documented & tested):** Mutually recursive `use` imports between two object modules remain rejected by the loader's acyclic module-graph invariant (use `ObjectId` or a shared module for cyclic relationships).

### 4. `guard(sim, is_autonomous)` on `interaction`
- Interaction guards accept an optional second boolean parameter `guard(sim, is_autonomous)` (rejected on `functional` guards), emitting `(args: 1)` on the guard tree and lowering `is_autonomous` to `param[0] == 1` (`!is_autonomous` to `param[0] != 1`), matching `sims-vm`'s autonomous probe (`args[0] = 1`) vs player pie menu (`args[0] = 0`). Works seamlessly with `lock:` clause composition.

### 5. Object Lifecycle Hooks `load`, `reset`, `cleanup`
- `object` and `template` support `load { ... }` (OBJf 2), `reset { ... }` (OBJf 3), and `cleanup { ... }` (OBJf 4), composing template blocks before object blocks and supporting `@id(...)` pinning.
- `TreeLowerer` lazily preserves incoming `stack_obj` (e.g., the cancelling Sim ID in `cleanup`) into `__saved_stack_obj` whenever an `ObjectSelf` lifecycle hook performs a foreign object access, restoring `stack_obj` cleanly after every foreign read/write/loop.

### 6. Compact Compound Arithmetic & Bitwise Expressions
- Left-associative arithmetic/bitwise chains (`(a * 24) / 5 + 1`) lower directly in-place into the destination register when safe (zero wasted scratch locals).
- Safety guards automatically fall back to a scratch local when:
  - Any RHS subexpression may alias or clobber the destination (comparing canonical VM cells via `canonical_air_cell`, treating foreign accesses as aliasing `stack.*`, `stack_obj`, `my.attr[*]`, and `my.data[*]`).
  - The expression contains `/` or `%` without a known non-zero constant divisor on a non-fresh destination (preventing partial writes on divide-by-zero traps).
  - The destination has VM write side effects or clamping (`*.motive*`, `*.person*`, `*.data[*]`, `global[*]`).

## `open-pack/` Migration & Results

Migrated 6 objects (`open-wall-phone.antics`, `open-double-bed.antics`, `open-meal-plate.antics`, `open-counter.antics`, `open-stove.antics`, `open-fridge.antics`):
- **Zero** `raw {}`, `@id(...)`, `@guard_id(...)`, `attr[...]`, `data[...]`, `temp[...]`, manual `strings 301`, or duplicate magic GUIDs remain anywhere in `open-pack/` (enforced by `on_disk_open_pack_is_pure_v2_expands_all_20_objects_and_passes_safety`).
- **Line count:** `1,410 -> 1,419` lines across all 25 `.antics` files in `open-pack/` (`1,709 -> 1,419`, `-17.0%` cumulative across PR A + PR B, with the +9 lines vs PR A reflecting explicit `enum CallOutcome` and `bitflags BedSide` type declarations replacing untyped integer/bit constants).
- **Contract note:** Auto-interning `open-wall-phone.antics` keeps `"Open Wall Phone"` at `STR# 301[0]` while ordering remaining internal dialog strings by encounter order; typed `functional` returns standardize on `temp[0]` across `OpenOakCounter`/`OpenHearthStove`/`OpenFrostFridge`, protected from mixed-lot ABI mismatch by runtime GUID filtering in `find_best_object::<T>`.

## Verification & Sol Reviews

- **Sol Round 1 (`gpt-6.2-sol`, `high` effort):** Found 4 P0 (typed `find_best_object` unfiltered search, `create_object` `FROM_RECORD` / untyped `run_functional` nominal manufacturing, canonical-cell & `ObjectSelf` aliasing in in-place arithmetic, `is_pure_single_operand` on `&mut` foreign fields in `while f.is_empty()`), 8 P1 (raw dialog `flags` bypass, implicit `temp[0]` clobber, enum zero-init & exhaustiveness, inconsistent lexical type lookup, multi-tile schema `lead_guid` & `Self`, lifecycle `stack_obj` clobber, trap-atomicity / `global[3]` side effects, Open Pack ABI note), and 3 P2 (duplicate enum/bitflag values & dialog arity/wildcard ordering, mutual import cycle doc/test, boundary VM tests). All fixed with dedicated live-VM and diagnostic tests.
- **Sol Round 2 (`gpt-6.2-sol`, `medium` effort):** **0 P0**, 3 P1 (fallback guard/zero-score check on `find_best_object::<T>`, implicit multi-tile lead matching `compile.rs`, staging side-effecting `*.data[*]` writes in compound arithmetic), 4 P2 (`run_functional::<T>` checked before binding annotation, `lookup_object_schema` respecting `in_object_scope`, shared reserved-type check on objects & import aliases, standalone `buttons:` arity for kinds 3/4). All fixed with `tier3_sol_r2_regressions`.
- **Local Release Gates:**
  - `cargo fmt --all -- --check`: PASS
  - `cargo clippy --workspace --all-targets -- -D warnings`: PASS
  - `cargo test --release -p sims-antics`: PASS (80/80 tests, including 31/31 `v2_compiler`, 9/9 `open_pack_behavioral`, 3/3 `open_pack_household_sim`)
  - `cargo test --release -p sims-game --test cheats_cache_and_open_pack`: PASS (7/7 tests)
