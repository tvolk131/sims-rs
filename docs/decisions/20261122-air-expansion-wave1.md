# `.air` (Antics IR) Expansion — Wave 1 & Removal of the All-or-Nothing Fallback Cliff

- **Date:** 2026-11-22
- **Status:** Accepted (PR 1 of 2 of `.air` expansion; `.antics` v2 left untouched)
- **Scope:** `crates/sims-antics/` (`ast.rs`, `parser.rs`, `lower.rs`, `lift.rs`, `structure_lift.rs`, `verify.rs`, `bin/sims-antics.rs`, tests) + `docs/decisions/`

## Context

Prior to this change, `sims-antics` structured `.air` lifting had two sharp limitations when inspecting real Sims 1 Complete Collection `BHAV` trees:

1. **All-or-nothing fallback cliff:** A single unmodeled primitive opcode or dirty Edith padding byte anywhere in a reachable node aborted structured lifting for the entire tree (`StructuredLiftOutcome::UnsupportedOpcode`), dumping the whole tree as raw hex `op 0xNNNN (...)` with `@n0:` labels.
2. **Undecoded irreducible CFGs & limited primitive coverage:** Genuinely irreducible CFGs fell back to raw hex `op 0xNNNN (...)` for every instruction (even clean expressions and subroutine calls), and only 18 primitive opcodes were modeled in `.air` (leaving out high-frequency primitives like `Action String`, `Test Object Type`, `Push Interaction`, `Run Tree By Name`, `Refresh`, `Relationship`, `Go To Relative Position`, `Set Headline`, `Notify Idle`, `Manage Inventory`, `Test Sim Interacting With`, `Grab`, `Drop`, `Drop Onto`, `Change Suit`, and `Find Location`, plus valid sub-modes on `Route To`, `Snap`, `Find Best Object`, `Play Sound`, `Dialog`, and `Animate`).

Only `14,692 / 28,690` (51.2%) Complete Collection `BHAV`s lifted structurally.

## Decision

### 1. Remove the All-or-Nothing Fallback Cliff in `.air`

- **Inline `op 0xNNNN (...)` inside structured blocks (`ast.rs`, `parser.rs`, `lower.rs`, `structure_lift.rs`):**
  - Added `Stmt::InlineRawOp` (linear statement `op 0xNNNN (b0, ..., b7);`) and `Condition::RawOp` (branching condition `if op 0xNNNN (...) { ... }`, `if !op 0xNNNN (...) { ... }`, `while op 0xNNNN (...) { ... }`).
  - Kept `Stmt::RawOp` with mandatory explicit `-> true: ..., false: ...` branches untouched so `.antics` v2 (`crates/sims-antics/src/v2/`) required zero changes.
  - Parser accepts both comma-separated `(0x01, 0x02, ...)` and space-separated `(0x01 0x02 ...)` operand bytes.
  - Whenever a single instruction is unmodeled, has an out-of-frame local/param index, or carries dirty Edith padding bytes under `strict_padding: true`, only that instruction falls back to an inline `op 0xNNNN (...)`; the surrounding `if`/`else`/`while` control flow is preserved.
- **Decoded instructions in irreducible / labeled CFGs (`lift.rs`, `structure_lift.rs`, `lower.rs`, `bin/sims-antics.rs`):**
  - `format_bhav_labeled_air_with_arities(chunk, opts, callee_arities)` (with `format_bhav_labeled_air` as the arity-free default) keeps `@n0:` labels and explicit `-> true: ..., false: ...` targets while decoding every clean modeled instruction into readable `.air` primitive / `expr(...)` / `call ...` syntax. Under `--relax-padding`, instruction records consumed by extended (`> 4`-arg) inline calls — or following unknown-arity inline calls — remain protected with `strict_padding: true` so extended argument bytes are never altered.
  - `lift_bhav_structured_with_arities` similarly refuses structured reordering before a known `> 4`-arg inline call or an unknown-arity inline call in any multi-instruction tree.
  - `format_bhav_raw_air(chunk)` preserves pure hex `op 0xNNNN (...)` output for `sims-antics lift --raw`.
  - `sims-antics lift --verify-equiv` compiles and verifies both structured output and decoded labeled-CFG fallback output.
  - `lower.rs` treats verbatim explicit-branch trees (`is_verbatim_explicit`) as 1-to-1 instruction streams supporting `> 251` instructions and preserving verbatim header `args`/`locals` when all decoded operand accesses are in-bounds.
- **Strict padding default + `--relax-padding` toggle (`LiftOptions`):**
  - `LiftOptions { strict_padding: true }` (default): decodes an instruction only when unused/reserved operand bytes are zero (preserving 100% operand byte-exactness and 100% full-BHAV byte-exactness in labeled-CFG mode).
  - `LiftOptions { strict_padding: false }` (exposed via `sims-antics lift --relax-padding`): strips verified-ignored padding bytes (cross-checked against `crates/sims-vm/src/primitives/*.rs` and `verify.rs::canonicalize_primitive_operands`) for cleaner inspection while preserving bounded trace equivalence.
- **Numeric indices in `.air` (Option A):** `my.motive[5]`, `my.person[58]`, `my.data[25]`, etc. remain strictly numeric in `.air`. (Symbolic annotations/comments are planned for PR 2.)

### 2. Wave 1 Primitives & Existing Sub-Mode Fixes

Every operand byte, unused padding mask, frame read/write side effect, and `is_always_true_instruction` classification was verified directly against `crates/sims-vm/src/primitives/*.rs`:

- **Existing primitive sub-mode fixes:**
  - `route_to` (`0x002d`, opcode 45): added `source: u16` (`0` param, `1` object slot default, `2` global slot).
  - `snap` (`0x002e`, opcode 46): added `mode: u16` (`0` param, `1` object container 0, `2` front tile, `3` object slot default, `4` global slot).
  - `find_best_object` (`0x000e`, opcode 14): added optional `origin: OperandRef` when `flags & 8 != 0`.
  - `play_sound` (`0x0017`, opcode 23): added `flags: u8` (`play_sound(id)`, `play_sound(id, stack_obj)`, `play_sound(id, flags: f)`) so non-standard Edith sound flag bits round-trip byte-exactly under `strict_padding: true`.
  - `dialog` (`0x0024`, opcode 36), `animate` (`0x002c`, opcode 44), `create_object` (`0x002a`, opcode 42), `set_to_next` (`0x001f`, opcode 31), `idle_for_input` (`0x0011`, opcode 17): aligned `strict_padding` vs `--relax-padding` and `verify.rs::canonicalize_primitive_operands`.
- **16 new Wave 1 `.air` primitives:**
  1. `0x0032` (50) `action_string(table:, index:)` (`action_string.rs`, always-true)
  2. `0x0020` (32) `test_object_type(guid, object)` (`object_type.rs`)
  3. `0x000d` (13) `push_interaction(menu:, object:, priority:, flags:[, icon:])` (`push.rs`)
  4. `0x001c` (28) `run_tree_by_name(table:, index:, mode:[, global: true])` (`named_tree.rs`)
  5. `0x0007` (7) `refresh(target:, mode:)` (`refresh.rs`, always-true)
  6. `0x001a` (26) `relationship(column:, dir:, flags:, value:[, local:])` (`relationship.rs`, always-true when `flags & 1 == 0`; read write-back modeled in `verify.rs` dataflow, lock prover, and equivalence checker)
  7. `0x001b` (27) `go_to_relative(location:, facing:, options:)` (`go_to_slot.rs::relative_slot`)
  8. `0x0029` (41) `set_headline(kind:, index:, duration:, options:[, stack_obj: true][, local:])` (`headline.rs`, always-true)
  9. `0x0031` (49) `notify_idle()` (`notify.rs`, always-true)
  10. `0x0033` (51) `manage_inventory(op:, kind:, flags:, guid:)` (`inventory.rs`)
  11. `0x0025` (37) `test_sim_interacting_with([person])` (`interacting.rs`)
  12. `0x0004` (4) `grab()` (`grab.rs`)
  13. `0x0005` (5) `drop()` (`drop.rs`)
  14. `0x002b` (43) `drop_onto(src, dst)` (`drop_onto.rs`, literal or `param[N]`)
  15. `0x0006` (6) `change_suit(index:, scope:[, remove: true])` (`change_suit.rs`, always-true)
  16. `0x0010` (16) `find_location(mode:, flags:[, local:])` (`find_location.rs`, always-true when `mode == 1`)

## Sample `.air` Output (Synthetic Examples)

### Structured `.air` with Wave 1 primitives & inline `op` fallback

```air
tree 4096 "SyntheticSample" (format: 0x8002, kind: 0, args: 1, locals: 1, revision: 0x00000000) {
    action_string(table: 300, index: 1);
    if test_object_type(0x12345678, stack_obj) {
        if route_to(slot: 0, source: 0, flags: 1) {
            if grab() {
                drop_onto(0, param[0]);
            }
        }
    }
    if op 0x0042 (0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08) {
        notify_idle();
        return true;
    }
    return false;
}
```

### Decoded Labeled-CFG `.air` (for genuinely irreducible CFGs)

```air
tree 4097 "IrreducibleSample" (format: 0x8002, kind: 0, args: 0, locals: 0, revision: 0x00000000) {
    @n0:
    expr(my.attr[0] == 0) -> true: @n1, false: @n2;
    @n1:
    expr(my.attr[1] == 0) -> true: @n2, false: true;
    @n2:
    expr(my.attr[2] == 0) -> true: @n1, false: false;
}
```

## Verification Results (In-Memory Complete Collection Corpus)

All corpus checks ran strictly in memory with `SIMS_INSTALL` (0 bytes written to disk, metadata-only summaries):

| Metric | Before | After (Wave 1) |
|---|---|---|
| Byte-exact labeled-CFG `.air` round-trip (`install_roundtrip`) | 28,690 / 28,690 (raw hex) | **28,690 / 28,690 (100.0%, decoded labeled CFG)** |
| Structurally lifted BHAVs (`structured_equiv`) | 14,692 / 28,690 (51.2%) | **25,941 / 28,690 (90.4%)** (`+11,249`, `+76.6%`) |
| Pure zero-raw-op structured (`strict_padding: true`) | 14,692 | **20,017** |
| Structured with inline `op 0xNNNN` (`strict_padding: true`) | 0 (fell off cliff) | **5,924** |
| Pure zero-raw-op structured (`--relax-padding`) | N/A | **23,890** |
| Decoded labeled-CFG fallback (`unstructured` + `unsupported_header_or_target`) verified `Equivalent` | 0 | **2,749 / 2,749 strict + 2,749 / 2,749 relaxed (100.0%)** |
| Formal bounded-trace-equivalence across all 28,690 BHAVs (`compile_failures` / `counterexamples` / `inconclusive`) | `0 / 0 / 0` (14,692 checked) | **`0 / 0 / 0`** (**28,690 / 28,690**, strict & relaxed) |
| `Fridges.iff` (`Objects.far`, 51 BHAVs) — structured vs decoded labeled-CFG | 26 structured / 25 raw | **48 / 51 structured** + **3 / 51 decoded labeled-CFG** with catalog arities (`41 / 51` structured + `10 / 51` decoded labeled-CFG standalone; **51 / 51 `Equivalent`** strict & `--relax-padding`) |
