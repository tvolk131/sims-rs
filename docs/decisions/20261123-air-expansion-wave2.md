# `.air` (Antics IR) Expansion — Wave 2: 49/49 Primitives, `break`/`continue` Loop Structuring, & Optional Symbol Comments

- **Date:** 2026-11-23
- **Status:** Accepted (PR 2 of 3 of Track 7 `.air` expansion; `.antics` v2 left untouched)
- **Scope:** `crates/sims-antics/` (`ast.rs`, `parser.rs`, `lower.rs`, `lift.rs`, `structure_lift.rs`, `verify.rs`, `lib.rs`, `bin/sims-antics.rs`, tests) + `docs/decisions/`

## Context

Wave 1 (`docs/decisions/20261122-air-expansion-wave1.md`) removed the all-or-nothing structured-lift fallback cliff and expanded `.air` to 34 named primitives, lifting `25,941 / 28,690` (90.4%) of Sims 1 Complete Collection `BHAV`s structurally and `48 / 51` in `Fridges.iff`.

Three gaps remained in `.air`:

1. **15 remaining Sims 1 primitives** (`0x0003`, `0x0009`, `0x000a`, `0x000b`, `0x000c`, `0x000f`, `0x0013`, `0x0015`, `0x0016`, `0x0018`, `0x001e`, `0x0021`, `0x0022`, `0x0023`, `0x002f`) still appeared as inline `op 0xNNNN (...)` bytes rather than named `.air` calls.
2. **Loop structuring** only emitted header-tested `while cond { ... }` loops, leaving natural loops with statement heads, multi-exit `break` escapes, `continue` back-edges, and infinite `Main` loops (with no `return` path) as labeled-CFG fallbacks (`2,748` unstructured trees across Complete Collection, including the remaining `3` BHAVs in `Fridges.iff`).
3. **Numeric-only readability:** Keeping `.air` indices strictly numeric (`my.motive[7]`, `my.person[58]`, `bcon[1]`, `call 4097(...)`, `action_string(table: 300, index: 1)`) preserves clean round-trip grammar, but human readers inspecting lifted `.iff`/`.far` files benefit from inline symbol comments.

## Decision

### 1. Complete 49 / 49 Named Sims 1 Primitive Coverage in `.air`

All 15 remaining executable primitives in `0..52` (opcodes `38, 39, 40` are the only 3 unused slots) now have first-class AST nodes, parser forms, lowerer encodings, strict & `--relax-padding` decoders, and `verify.rs` static/lock/equivalence models cross-checked directly against `crates/sims-vm/src/primitives/*.rs`:

| Opcode | `.air` Syntax | VM Reference | Notes |
|---|---|---|---|
| `0x0003` (3) | `find_best_interaction()` | `autonomy.rs` | Invokes TTAB check trees; disables closed-caller param inference & reports `InconclusiveProof` when reachable |
| `0x0009` (9) | `burn(mode:, flags:)` | `burn.rs` | `mode: u8`, `flags: u8` |
| `0x000a` (10) | `tutorial(mode)` | `events.rs` | Traps when `mode > 1` |
| `0x000b` (11) | `get_distance_to(temp[N][, from: <op>][, flags: f])` | `distance.rs` | Always-true; `from` reads `ops[3], ops[4..6]` when `flags & 1 != 0`; Global 34 modeled |
| `0x000c` (12) | `get_direction_to(<dst>[, from: <op>][, flags: f])` | `direction.rs` | Always-true; writes `<dst>` (tracked in static, lock, dataflow, and equiv models); Global 34 & scope 22 modeled |
| `0x000f` (15) | `breakpoint(<cond>)` | `debug.rs` | Always-true; reads operand `ops[0..2]` in scope `ops[2..4]`; Global 34 modeled |
| `0x0013` (19) | `make_new_character(skin:, age:, sex:, species:)` | `make_character.rs` | Writes `stack_obj` |
| `0x0015` (21) | `show_string(table:, index:)` | `debug.rs` | Always-true; `table: u16`, `index: u16` |
| `0x0016` (22) | `look_towards(mode)` | `look_towards.rs` | Always-true when `mode == 0`; traps when `mode > 3` |
| `0x0018` (24) | `old_relationship(write:, column:, param:, dir:, flags:)` | `relationship.rs` | Legacy opcode 24; always-true when `flags & 1 == 0`; write-back to `param[N]` modeled |
| `0x001e` (30) | `gosub_found_action()` | `found_action.rs` | Dynamic subroutine call; disables closed-caller param inference & reports `InconclusiveProof` when reachable |
| `0x0021` (33) | `find_worst_motives(target:, mode:)` | `worst_motives.rs` | Always-true; traps on `target == 2` or `mode > 2`; mutates `temp[0..=4]` |
| `0x0022` (34) | `ui_effect(mode:[, target: <op>][, flags: f])` | `ui_effect.rs` | Always-true; reads `target` only when `mode <= 2` |
| `0x0023` (35) | `special_effect(duration:, kind:, size:, flags:, str_index:)` | `special_effect.rs` | Always-true; `duration: i16`, `kind: u8`, `size: u8`, `flags: u8`, `str_index: u8` |
| `0x002f` (47) | `reach(mode:, moves:, slot_param:)` | `reach.rs` | `mode == 1` reads `param[slot_param]`; dynamic callback modeled |

Numeric indices remain strictly numeric (`my.motive[7]`, `my.person[58]`, `my.data[25]`).

### 2. Zero-Overhead `while true`, `break`, `continue`, & Loop-DAG Structuring

- **`Stmt::Loop { span, body }` (`while true { ... }`):** Added zero-overhead unconditional loop lowering in `lower.rs` using sentinel back-edge patching so `while true { ... }` emits **zero extra dummy instructions** (emitting a 1-node `0 == 0` self-loop only for empty/pure-`continue` bodies). This preserves exact step-for-step trace equivalence even on trees touching Global 34 or `ui_effect` where `verify.rs` disables silent-step elision.
- **Natural loops with `break` and `continue` (`structure_lift.rs`):**
  - Tracks active loop context `LoopCtx { header, exit }` so back-edges to `header` emit `continue;` and jumps to the selected canonical loop exit `exit` emit `break;`.
  - Picks a canonical loop exit `E` (preferring an internal node over terminal `return`) and defers header-tested `while cond` into `while true` when the header exits to `return` while the body escapes to a real node `E`, avoiding tail duplication.
  - Computes per-loop DAG immediate post-dominators (`compute_loop_ipdoms`, ignoring back-edges and treating loop exits as DAG sinks) so diamonds inside infinite `Main` loops (which have no `return` path) merge cleanly instead of exponentially duplicating tails.
  - Flattens immediate-terminator branches (`if cond { return/break/continue; }`) to reduce nesting depth, and caps recursion depth at `28` (strictly below the parser's `MAX_NESTING_DEPTH = 64`).

### 3. Optional Readability Symbol Comments (Option A — Comments Only, Zero Grammar Risk)

- Extended `LiftOptions` with `annotate_comments: bool` (defaults to `false` for noise-free programmatic callers; enabled by default in `sims-antics lift`, with `--no-comments` to disable; always `false` in `--list`).
- Added `LiftSymbols`, `extract_lift_symbols_from_iff`, `lift_bhav_structured_with_context`, and `format_bhav_labeled_air_with_context` (strictly in-memory / stdout-only; never written to disk):
  - **Private sibling BHAV labels:** `call 4096..=8191(...)` -> `// "<label>"`.
  - **Local `BCON` values & table labels:** `bcon[...]` -> `// bcon[table:entry] = <val> ("<label>")`.
  - **Local `STR#` previews:** `action_string`, `dialog`, `run_tree_by_name`, `show_string` -> `// "<preview>"` (English language 1, sanitized single-line, control chars stripped, truncated to 48 chars).
  - **Canonical VM field names:** `my.motive[N]` / `stack.motive[N]` / `set_motive_change`, `my.person[N]` / `stack.person[N]`, and `my.data[N]` / `stack.data[N]` -> `// Hunger`, `// Age`, `// Lockout`, etc.
- Because the `.air` lexer skips `// ...` comments, annotated `.air` compiles cleanly through `compile_air` and proves `Equivalent` across the entire corpus.

## Sample `.air` Output (Synthetic Example)

```air
tree 4096 "SyntheticMainLoop" (format: 0x8002, kind: 0, args: 0, locals: 1, revision: 0x00000000) {
    while true {
        if my.motive[7] < bcon[0] { // Hunger, bcon[0:0] = -20 ("Tuning")
            action_string(table: 300, index: 1); // "Grab Snack"
            get_direction_to(local[0], from: stack_obj);
            look_towards(0);
            call 4097(0, 0, 0, 0); // "open"
            continue;
        }
        if my.data[25] != 0 { // Lockout
            break;
        }
        idle(30);
    }
    return true;
}
```

## Verification Results (In-Memory Complete Collection Corpus)

All corpus checks ran strictly in memory with `SIMS_INSTALL` (0 bytes written to disk, metadata-only summaries):

| Metric | Pre-Wave 1 | Wave 1 (PR #396) | Wave 2 (This PR) |
|---|---|---|---|
| Distinct modeled `.air` primitives | 18 | 34 | **49 / 49 (100%)** |
| Byte-exact labeled-CFG `.air` round-trip (`install_roundtrip`) | 28,690 / 28,690 (raw) | 28,690 / 28,690 (decoded) | **28,690 / 28,690 (100.0%, decoded)** |
| Structurally lifted BHAVs (`structured_equiv`) | 14,692 / 28,690 (51.2%) | 25,941 / 28,690 (90.4%) | **27,781 / 28,690 (96.83%)** (`+1,840` vs Wave 1) |
| Pure zero-raw-op structured (`strict_padding: true`) | 14,692 | 20,017 | **22,988** (`+2,971`) |
| Structured with inline `op 0xNNNN` (`strict_padding: true`) | 0 | 5,924 | **4,793** (`-1,131`) |
| Pure zero-raw-op structured (`--relax-padding`) | N/A | 23,890 | **27,522** (`99.07%` of structured) |
| Decoded labeled-CFG fallback (`unstructured` + `unsupported_header_or_target`) | 0 | 2,749 / 2,749 | **909 / 909** (`1` header/target + `908` irreducible) |
| Comment-annotated `.air` verified `Equivalent` (`annotate_comments: true`) | N/A | N/A | **28,690 / 28,690 (100.0%)** |
| Formal bounded-trace-equivalence (`compile_failures` / `counterexamples` / `inconclusive`) | `0 / 0 / 0` (14,692) | `0 / 0 / 0` (28,690) | **`0 / 0 / 0`** (**28,690 / 28,690**, strict, annotated & relaxed) |
| `Fridges.iff` (`Objects.far`, 51 BHAVs) — with catalog arities | 26 / 51 | 48 / 51 structured + 3 labeled-CFG | **51 / 51 (100.0%) structured** (`50 / 51` pure zero-raw-op relaxed) |
| `Fridges.iff` standalone CLI (`sims-antics lift`) | 26 / 51 | 41 / 51 structured + 10 labeled-CFG | **43 / 51 structured + 8 labeled-CFG** (**51 / 51 `Equivalent`** strict, relaxed, & `--no-comments`) |
