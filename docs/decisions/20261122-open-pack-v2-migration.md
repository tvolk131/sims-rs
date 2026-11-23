# ADR: `open-pack/` Migration to `.antics` v2 & Retirement of `object.toml`

- **Date:** 2026-11-22
- **Status:** Accepted (PR 2 of 2 — Full `open-pack/` v2 Migration)
- **Scope:** `open-pack/`, `crates/sims-antics/`
- **Related:**
  - `docs/decisions/20261122-antics-v2-language-and-air-ir.md` (PR 1 of 2 — `.air` IR split & `.antics` v2 compiler)
  - `docs/decisions/20261122-sims-antics-verifier-and-standalone-pack.md`

---

## Context

PR 1 (`#390`) established the clean two-layer architecture separating high-level `.antics` v2 authoring from explicit low-level `.air` (Antics IR), while keeping the 20 clean-room `open-pack/` objects on their legacy `objects/<slug>/{object.toml, behavior.antics}` layout.

Migrating all 20 objects to `.antics` v2 completes the transition:
- eliminates all 20 redundant `object.toml` manifests so each object has a single source of truth,
- removes manual `my.attr[i]` / `stack.attr[i]`, `stack_obj` save/restore, lock acquire/release, and `my.person.priority == 0` cancel boilerplate,
- extracts genuinely shared behavioral helpers and reusable templates across object families, and
- keeps `open-pack/pack.toml`, `open-pack/surfaces.toml`, and `open-pack/starter-house.toml` as pure world/pack TOML manifests.

---

## Decision

### 1. Flat Single-File Object Layout & Reusable Modules

All 20 `open-pack/` objects now live as flat, single-file `.antics` v2 sources under `open-pack/objects/<slug>.antics`, importing shared building blocks via `use crate::...;` (ensuring pack directories remain relocatable even when copied under arbitrary directory names):

```text
open-pack/
├── pack.toml                   # Pack manifest (version 0.4.0, 20 object slugs)
├── surfaces.toml               # Procedural clean-room floors & walls
├── starter-house.toml          # 5-room starter cottage + 2-Sim household
├── common/
│   └── helpers.antics          # 6 shared `pub fn` helpers (motive loops, toggles, wear, repairs)
├── templates/
│   ├── bed.antics              # `pub template BedBase` (open-cot, open-double-bed)
│   ├── seat.antics             # `pub template SeatBase` (open-bench, open-dining-chair, open-sofa)
│   ├── plumbing.antics         # `pub template PlumbingFixture` (open-shower, open-toilet)
│   └── toggle.antics           # `pub template ToggleableDevice` (open-tv, open-floor-lamp)
└── objects/
    └── open-*.antics           # 20 self-contained v2 object files (0 object.toml, 0 raw blocks, 0 @id)
```

### 2. Template & Helper Reuse Summary

| Shared Module / Template | Exported Items | Consumers in `open-pack/` |
|---|---|---|
| `common/helpers.antics` | `restore_motive_loop`, `restore_two_motives_loop`, `set_toggle_state`, `record_fixture_wear`, `repair_by_progress`, `run_timed_cycles` | `templates/bed`, `templates/seat`, `open-bookshelf`, `open-floor-lamp`, `open-front-door`, `open-shower`, `open-sofa`, `open-toilet`, `open-tv` |
| `templates/bed.antics` | `BedBase` | `open-cot`, `open-double-bed` |
| `templates/seat.antics` | `SeatBase` | `open-bench`, `open-dining-chair`, `open-sofa` |
| `templates/plumbing.antics` | `PlumbingFixture` | `open-shower`, `open-toilet` |
| `templates/toggle.antics` | `ToggleableDevice` | `open-floor-lamp`, `open-tv` |

All 20 objects compile with:
- **0 `object.toml` files** anywhere in `open-pack/`,
- **0 `raw { ... }` blocks** (pure high-level v2 syntax throughout),
- **0 `@id(...)` / `@guard_id(...)` annotations** (deterministic automatic BHAV allocation),
- **multi-graphic off/on sprite states** on `open-floor-lamp`, and
- **structural safety guarantees** (`with lock(...)`, LIFO `defer { ... }` rollback on `open-fridge` and multi-tile cleanup on `open-double-bed`, `repeat`/`while ... cancel_on_abort`, automatic `stack_obj` preservation across `for ... in objects_by_guid(...)` / `self.parts()` / `run_functional(...)`).

### 3. Compiler Enhancement: Constant-Argument Helper Inlining

During migration, passing literal booleans/constants into `pub fn set_toggle_state(obj: Object, flag: &mut bool, enabled: bool, sfx: Sound)` revealed that unconditionally spilling immutable literal arguments into fresh BHAV locals (`let __arg_enabled_0 = 0; stack.attr[1] := __arg_enabled_0;`) obscured constant release writes (`:= 0`) from the static lock/toggle prover in `verify.rs` and wasted BHAV locals.

`crates/sims-antics/src/v2/lower.rs` now binds immutable, non-mutated/non-borrowed `i16` and `bool` literal/BCON arguments directly as `Binding::ConstI16` / `Binding::ConstBool` during helper inlining, eliminating wasted local allocations and preserving exact constant-write visibility for `verify_object_safety` (while still spilling any parameter that is mutated, borrowed via `&mut`, or shadowed inside the callee).

---

## Before / After Line-Count Comparison

| Component | Before (v1 `object.toml` + `behavior.antics`) | After (v2 `.antics`) | Delta |
|---|---:|---:|---:|
| Legacy `object.toml` files (20) | 1,019 lines | 0 lines (retired) | -1,019 |
| Legacy `behavior.antics` files (20) | 1,629 lines | 0 lines (retired) | -1,629 |
| Shared `common/helpers.antics` (1) | — | 94 lines | +94 |
| Shared `templates/*.antics` (4) | — | 113 lines | +113 |
| Migrated `objects/*.antics` (20) | — | 1,506 lines | +1,506 |
| **Total object source lines** | **2,648 lines (40 files)** | **1,713 lines (25 files)** | **-935 lines (-35.3%)** |
| World manifests (`pack.toml`, `surfaces.toml`, `starter-house.toml`) | 365 lines | 365 lines | 0 |
| **Total `open-pack/` lines** | **3,013 lines (43 files)** | **2,078 lines (28 files)** | **-935 lines (-31.0%)** |

---

## Verification

- **20 / 20** `open-pack/` objects pass `verify_pack_safety` with **0 violations**.
- **20 / 20** objects expand cleanly via `expand_v2_to_air` and `sims-antics expand <file> --pack open-pack` into valid `.air` that compiles via `compile_air` with matching BHAV counts.
- **100% behavioral & format parity:** `open_pack_behavioral`, `open_pack_household_sim` (0 VM traps, all 8 motives, Cooking/Mechanical skills, bills, cancel safety), `open_objects_vm`, `v2_compiler`, `cli_lift`, `install_roundtrip` (`28,690 / 28,690`), `structured_equiv` (`14,692 / 14,692`), and `sims-game --test cheats_cache_and_open_pack` (`7 / 7` both with and without `SIMS_INSTALL`).
- `unsafe_code = "forbid"`, zero new third-party crates, 100% deterministic `BTreeMap`/`BTreeSet` compilation, and zero commercial data written to disk or committed.
