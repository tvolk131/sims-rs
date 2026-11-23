# ADR: Formal Safety & Trace-Equivalence Verifier (`sims-antics::verify`) and Complete 20-Object Clean-Room Standalone `open-pack/`

- **Date:** 2026-11-22
- **Status:** Accepted (Phase 3 implemented on `antics/formal-verifier-and-standalone-pack`)
- **Scope:** `crates/sims-antics` (`verify.rs`, `structure_lift.rs`, `house.rs`, `sim_pack.rs`, CLI `verify`/`equiv`/`build`), `open-pack/` (20 clean-room objects, `surfaces.toml`, `starter-house.toml`), and in-memory `SIMS_INSTALL` structured-equivalence verification.

## 1. Context & Motivation

Phases 1 and 2 established `sims-antics` as a zero-dependency SimAntics compiler, IFF/FAR binary writer, procedural isometric sprite/WAV pipeline, and 10-object `open-pack/` source tree. Before wiring dual-mode standalone boot into `sims-game`, two foundations were needed:

1. **Formal Verification for Clean-Room Scripts & Compiler Soundness:**
   - Clean-room objects written in `.antics` must never trap the VM, reference out-of-bounds `BCON`/`STR#`/`SLOT`/`FWAV`/local/param entries, or leak occupancy locks on early returns or mid-loop player cancellation (`my.person.priority == 0`).
   - Beyond verbatim bytecode round-tripping (`install_roundtrip`: 28,690/28,690 Complete Collection BHAVs), we want a formal, pure-Rust **behavioral trace-equivalence verifier** capable of proving that high-level structured `.antics` programs (`if`/`else`, `while`, named expressions, readable primitives, `call`) lower to semantically equivalent SimAntics bytecode — even when Edith node numbering, dead nodes, unused operand padding bytes, or comparison polarities differ.
2. **Complete Clean-Room Standalone Household & World Pack:**
   - A playable standalone household requires furniture covering all 8 Sim motives (**Hunger, Comfort, Hygiene, Bladder, Energy, Fun, Social, Room**), skill study, architectural entry/lighting/cleanliness/bills utility, procedural floor (`.flr`) and wall (`.wll`) patterns, and a valid starter `House01.iff` + `Neighborhood.iff` with a starter 2-Sim household.

## 2. Clean-Room Legal Boundary

- **Zero commercial data on disk or in git:** All 20 `open-pack/` objects, 4 `.flr` floors, 4 `.wll` walls, and the starter cottage & 2-Sim household (`Alex Sunvale` & `Sam Sunvale`) are 100% original clean-room assets in the `0x7F00_0001..=0x7F00_01FF` namespace.
- **Strictly in-memory `SIMS_INSTALL` verification:** `crates/sims-antics/tests/structured_equiv.rs` skips cleanly when `SIMS_INSTALL` is unset. When `SIMS_INSTALL` is set, it reads commercial `.iff` files in memory, lifts structured BHAVs into high-level `.antics` in memory using synthetic labels (`Tree_<id>`), recompiles them in memory, runs `check_bhav_equivalent` in memory, reports metadata-only counts, and drops all buffers without ever writing to disk.

## 3. Formal Safety & Trace-Equivalence Verifier (`crates/sims-antics/src/verify.rs`)

Built in pure deterministic safe Rust (`unsafe_code = "forbid"`, `BTreeMap`/`BTreeSet` only, zero external solver crates):

### 3.1 Static & Symbolic Safety Prover (`verify_object_safety`, `verify_pack_safety`, CLI `sims-antics verify <pack_dir>`)
1. **Structural & Resource Bounds Checks:** Validates all `OBJD`/`OBJf`/`TTAB`/`TTAs`/`DGRP`/`SPR2`/`PALT` links, verifies zero dangling jumps and zero unreachable nodes, validates private `BCON` (`4096..=4159`), `STR# 301` dialog indices, `SLOT` indices, `FWAV` sound IDs, local/param frame dimensions, known primitive opcodes, private subroutine existence & arity, and no-trap `Expression` invariants (writable LHS, no constant divide/modulo by zero, valid flag bit `0..=16`).
2. **Interprocedural Symbolic Lock & Cancel-Safety Prover:** Identifies scoped occupancy locks (both scalar `stack.attr[k] := 1 / 0` and bitmask `flag_set / flag_clear`, distinguishing scoped mutexes from multi-interaction persistent toggles such as TV power or lamp on/off) and symbolically explores the CFG across private subroutine calls, loop iterations (with counter widening), early `break`/`return false`, and `my.person.priority == 0` cancel branches. Every reachable exit must leave all acquired occupancy locks provably released; any leak reports an exact counterexample trace.

### 3.2 Behavioral Trace-Equivalence Checker (`check_bhav_equivalent`, CLI `sims-antics equiv <a.antics> <b.iff> --tree <id>`)
- Performs synchronized CFG product-graph bisimulation over symbolic/concrete `i16` frame-private state (`local`/`param`) and canonicalized external trace events:
  - Ignores dead/unreachable nodes, arbitrary node numbering/reordering, verified-ignored operand padding bytes (such as `Expression` byte 4 and trailing zeros on `Sleep`/`PlaySound`/`RemoveObject`/`RunFunctionalTree`/`AlterBudget`/`StopAllSounds`), and architecturally unreachable `on_false` targets on always-true primitives.
  - Canonicalizes pure `Expression` comparisons (polarity flips `!=`↔`!(==)`, `>=`↔`!(<)`, operand commutativity when Global 34 scratch-pointer aliasing does not apply, and integer boundary equivalents such as `x >= c` vs `x > c - 1`).
  - Steps silent no-op nodes and frame-private local calculations transparently while widening identical loop headers symmetrically so unbounded identical loops verify in $O(N)$ states.
  - Returns `Equivalent { states_checked, paths_verified }`, an exact `Counterexample { path, divergence }`, or `Inconclusive { reason }` if exploration bounds are exceeded (never false positives).

## 4. In-Memory Structured High-Level Lifter (`crates/sims-antics/src/structure_lift.rs`)

- Computes dominators (natural `while` loops with `break`/`continue`) and immediate post-dominators (`if`/`else` merge points) to lift raw `BHAV` bytecode into pure high-level `.antics` containing **zero raw `op 0xNNNN` instructions**.
- **In-memory Complete Collection Verification (`tests/structured_equiv.rs`):**
  - Scanned **28,690** real Complete Collection BHAVs strictly in memory (`0` bytes written to disk, metadata-only counts/numeric diagnostics), resolving global (`256..=4095`), private (`4096..=8191`), and semi-global (`8192..=65535`) call arities via `Catalog::build_with_pool` matching `sims-vm` dispatch.
  - Cleanly lifted **14,692** BHAVs into pure high-level structured `.antics` (**0 raw `op 0xNNNN` instructions**, **0 compile failures**).
  - Formally verified **14,692 / 14,692 (100.0%)** `Equivalent` via `check_bhav_equivalent_with_arities` with **0 `Counterexample`s** and **0 `Inconclusive`** (remaining: `13,206` using primitives outside the current high-level DSL subset, `1` kind-4 indexed-entry / `>4`-arg inline-call tree, `791` irreducible/unstructured CFGs).

## 5. Complete 20-Object Standalone `open-pack/` World Layout

### 5.1 20 Clean-Room Objects (`0x7F00_0001..=0x7F00_0090`)
- **Existing 10:** `open-bench` (`0x7F00_0001`), `open-cot` (`0x7F00_0002`), `open-basin` (`0x7F00_0003`), `open-double-bed` (`0x7F00_0010..=0x7F00_0012`), `open-fridge` (`0x7F00_0020`), `open-counter` (`0x7F00_0021`), `open-stove` (`0x7F00_0022`), `open-meal-plate` (`0x7F00_0023`), `open-wall-phone` (`0x7F00_0030`), `open-shower` (`0x7F00_0040`).
- **New 10:**
  1. `open-toilet` (`0x7F00_0050`): Bladder + flush SFX + clog/unclog state guards + Mechanical XP.
  2. `open-sofa` (`0x7F00_0051`): Comfort Relax, Energy Nap, and Social Chat with cancel-safe lock.
  3. `open-tv` (`0x7F00_0052`): Fun watch loop + persistent on/off power toggle.
  4. `open-bookshelf` (`0x7F00_0053`): Fun novel reading + Cooking & Mechanical skill study.
  5. `open-dining-table` (`0x7F00_0060`): Container surface + Room-boosting Set/Tidy Table.
  6. `open-dining-chair` (`0x7F00_0061`): Cancel-safe Comfort dining seating.
  7. `open-floor-lamp` (`0x7F00_0070`): Toggleable warm light + Room score boost.
  8. `open-trash-can` (`0x7F00_0071`): Fill level + Dispose & cancel-safe Empty Trash loop.
  9. `open-front-door` (`0x7F00_0080`): Architectural entry + Knock/Welcome Social + Lock/Unlock.
  10. `open-mailbox` (`0x7F00_0090`): Daily bills + `alter_budget` payment with insufficient-funds guard.

### 5.2 Procedural Surfaces, Starter Lot & 2-Sim Household (`house.rs`, `sim_pack.rs`)
- `open-pack/surfaces.toml` compiles 4 procedural `.flr` floor IFFs (`STR# 0` + `PALT 1` + 3 zoom `SPR2` chunks) and 4 procedural `.wll` wall IFFs (`STR# 0` + `PALT 1` + 3 zoom 4-frame `SPR#` chunks via `encode_legacy_spr`).
- `open-pack/starter-house.toml` compiles `House01.iff` (native case-sensitive `HOUS` #0/#1, `SIMI` #1, 14 `Arry` 64×64 grids including 4-byte height array #0 and dry `0xff` pool/water arrays #9/#10, `FLRm`, `WALm`, `objt` with sentinel non-zero signatures for fresh-load reset/Init, `ObjM` with all multi-tile furniture records + 2 `Person` records with 6-unit footprints, and traversable `[[doorway]]` openings connecting every room), self-contained starter Sim character `.iff`s (`characters/open_sim_01.iff`, `characters/open_sim_02.iff`), and `Neighborhood.iff` (`NGBH`, `NBRS`, `FAMI` v9, `FAMs`) for the starter 2-Sim `Sunvale Household` (`$20,000`). Validated end-to-end through `sims_content::House::parse_bytes`, `World::read_arrays`, `World::rebuild_rooms`, `World::read_neighbors`, and full `sims-vm` `read_types` → `read_objects` → `assign_rooms` → `visit_objects` startup booting both saved Sims organically onto `Main` (`#4097`).
- `compile_standalone_pack_dir` and `sims-antics build <pack_dir> --out <dir>` emit the complete standalone layout (`open-pack.far`, `objects/*.iff`, `sounds/*.wav`, `floors/*.flr`, `walls/*.wll`, `characters/*.iff`, `houses/House01.iff`, `neighborhood/Neighborhood.iff`) ready for dual-mode `sims-game` boot in the follow-up glue PR.
