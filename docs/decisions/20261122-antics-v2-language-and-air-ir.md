# ADR: `.antics` v2 High-Level Language & `.air` (Antics IR) Compiler Architecture

- **Date:** 2026-11-22
- **Status:** Accepted (PR 1 of 2 — Compiler, IR Split, CLI & Tests)
- **Crate:** `crates/sims-antics/`
- **Related:**
  - `docs/decisions/20261122-sims-antics-compiler-and-open-pack.md`
  - `docs/decisions/20261122-sims-antics-verifier-and-standalone-pack.md`

---

## Context

`sims-antics` originally unified low-level SimAntics bytecode inspection/lifting (`bcon 4096`, `tree 4096`, `my.attr[0]`, `stack.attr[0]`, `op 0xNNNN`, `trailing: [...]`, `raw(0xHH)`) and object authoring (`object.toml` + `.antics` behavior files) inside one surface syntax.

While that representation achieved:
- **28,690 / 28,690** byte-exact Complete Collection BHAV round-trips,
- **14,692 / 14,692** formally verified structured trace-equivalence proofs,
- zero-violation formal lock/cancel safety verification across all 20 clean-room `open-pack/` objects, and
- stdout-only `sims-antics lift` inspection,

authoring real Sims 1 objects directly in v1 `.antics` + `object.toml` exposed five ergonomic and safety pain points:
1. **Split source of truth:** Object metadata, interactions, slots, sounds, and tuning lived in `object.toml` while tree IDs, attribute indices, and sound IDs were manually mirrored in `.antics`.
2. **`my` vs `stack` footguns:** In SimAntics, `my` is the object during `Init`/`Main`, whereas `my` is the Sim and `stack` is the object during TTAB interactions and functional callbacks. `find_best_object`, `run_functional_tree`, and `create_object` silently overwrite `stack_obj`.
3. **Manual lock & rollback discipline:** Occupancy locks (`stack.attr[0] := 1` / `:= 0`), player-cancel loops (`my.person.priority == 0` before and after every `idle`), and multi-step rollback (refunding funds/stock on stove failure) had to be hand-maintained on every exit path.
4. **No code reuse:** Common patterns (rest/sit loops, toggle switches, repair loops) were duplicated across all 20 objects.
5. **Conflated IR vs authoring roles:** `sims-antics lift` printed low-level `my.attr[0]` / `stack.attr[0]` trees under the same `.antics` name as authored sources.

---

## Decision

We establish a clean **two-layer compiler architecture**:

```
  .antics (v2 high-level authoring)
          │
          │  v2/lexer -> v2/parser -> v2/loader -> v2/lower
          │  (templates, use imports, pub fn inlining, self/sim context,
          │   with lock, defer, repeat cancel_on_abort, short-circuit &&/||/!)
          ▼
  .air (Antics IR — explicit low-level textual IR)  ◄─── sims-antics lift (.iff/.far)
          │
          │  lexer -> parser -> lower -> encode -> verify_object_safety
          ▼
  .iff / .far + synthesized WAVs
```

### 1. Explicit `.air` (Antics IR) Layer
- The existing low-level syntax (`bcon`, `str`, `tree`, `interaction`, `my.attr[i]`, `stack.attr[i]`, `op 0xNNNN`, `trailing: [...]`, `raw(0xHH)`) is formally designated **`.air` (Antics IR)**.
- All existing APIs (`compile_antics`, `lift_bhav_structured`, `format_bhav_antics`, `check_bhav_equivalent*`, `verify_*`) remain intact alongside explicit `.air` aliases (`compile_air`, `format_bhav_air`, `lift_bhav_air_structured`).
- `sims-antics lift` clearly labels lifted output as `.air` (Antics IR), preserving all 28,690 round-trip and 14,692 structured-equivalence proofs untouched.

### 2. High-Level `.antics` v2 Authoring Language (`crates/sims-antics/src/v2/`)
A single-file, Rust-flavored object authoring language with zero external dependencies and deterministic `BTreeMap`/`BTreeSet` compilation:
- **Modules & `use` imports:** `use pack::module::Item;` and `use pack::module::{A, B};` with symlink-free pack-root containment, `pub fn`/`pub template` visibility enforcement, and cycle detection.
- **Reusable helper functions (`pub fn` / `fn`):** Typed parameters (`i16`, `bool`, `Sim`, `Object`, `ObjectId`, `Motive`, `Sound`, `Slot`, `&mut bool`, `&mut i16`, `&mut ObjectId`) with `*p` dereferencing and optional `-> bool` return. Lowered via hygienic inlining with fresh local allocation and recursion-cycle rejection, enabling true `&mut` state mutation and zero-overhead `Motive`/`Sound`/`Slot` specialization.
- **Templates & concrete objects:** `pub template Name { ... }` and `object "Display Name" [from Template] { ... }` with flat top-level declarations (no `state { ... }` wrapper):
  - `param name: i16 [= default];` / `param name: bool [= default];` (omitting a default makes the param required; `i16` params emit into `BCON #4096`, `bool` params constant-fold cleanly).
  - `state name: bool|i16|ObjectId [= init];` (deterministically allocates `attr[0..N]`, auto-sets `OBJD.num_attributes`, and auto-generates `Init`).
  - `sound name = preset("...");`, `slot name { ... }`, `strings [id] { ... }`, `sprite: Sprite::procedural("...", primary: #RRGGBB, ...)`, multi-graphic `graphics: [...]`, multi-tile `tiles: [...]`, `init { ... }`, `main { ... }`, `functional <kind> { ... }`, and `interaction "Label" { ... }`.
- **Context safety & automatic `stack_obj` preservation:** `self.<state>` always targets the object (`my.attr[i]` in `init`/`main`, `stack.attr[i]` in interactions/functionals) while `sim.*` targets the Sim (`my.*`) and is rejected in `init`/`main`. Whenever a tree invokes `find_best_object`, `run_functional`, `create_object`, `for` iteration, or foreign `obj.attr[i]`, the compiler automatically captures `let __self_obj = stack_obj;` and restores `stack_obj := __self_obj;` immediately after every foreign-object operation.
- **Safety-by-construction statements:**
  - `with lock(self.occupied) { ... }` sets the lock on entry and releases it on every exit path (fallthrough, `return`, `break`, `continue`, `cancel_on_abort`).
  - `defer { ... }` runs cleanup in strict LIFO order on every exit path of its lexical scope (rejecting control-flow escape out of `defer`).
  - `repeat <count> cancel_on_abort { ... }` and `while <cond> cancel_on_abort { ... }` emit loop-entry and pre/post-`sleep(...)` `my.person.priority == 0` abort checks.
  - Short-circuit `&&`, `||`, `!`, `return <bool_expr>;`, expression-bodied `guard(sim) => <bool_expr>`, and `raw { ... }` escape hatch for `.air`.
- **Inspectable lowering:** Every v2 `.antics` object lowers to readable `.air` text inspectable via `sims-antics expand <file.antics> [--pack <pack_dir>]` (stdout-only), and compiles through the exact `.air -> BHAV/IFF` encoder + `verify_object_safety` prover.

### 3. Two-PR Rollout Plan
- **PR 1 (this PR):** `.air` split, complete v2 `.antics` compiler/loader/CLI (`expand`, plus v2 support in `check`/`verify`/`equiv`/`build`), ADR, and comprehensive CI-safe unit/integration/VM tests (including v2 prototypes of Camp Cot, Retro TV, Floor Lamp, and Frost Fridge). Existing `open-pack/` (`object.toml` + legacy scripts) remains untouched and 100% green.
- **PR 2 (follow-up):** Migrate all 20 `open-pack/` objects to high-level `.antics` v2 with shared templates/helpers and retire legacy `object.toml`.

---

## Consequences

- **Positive:**
  - Object authors write concise, type-checked, context-safe `.antics` without manual `attr[i]` / `BCON` / `FWAV` / `BHAV` ID bookkeeping or `my`/`stack` inversion bugs.
  - Locks, rollbacks, and cancel-on-abort cleanup become structural guarantees that pass `verify_object_safety` out of the box.
  - Low-level bytecode inspection (`sims-antics lift`) and compiler debugging (`sims-antics expand`) share one canonical `.air` IR.
- **Constraints honored:**
  - `unsafe_code = "forbid"`, zero new third-party crates, 100% deterministic `BTreeMap`/`BTreeSet` iteration, zero commercial data written to disk or committed.
