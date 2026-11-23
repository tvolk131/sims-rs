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
- **multi-graphic lit/off sprite states** on `open-floor-lamp` (keeping the warm lit graphic at index `0` so boot `data[0] == 0` and catalog thumbnails preserve v0.3.0 parity while `Turn Off Light` switches to graphic `1`), and
- **structural safety guarantees** (`with lock(...)`, LIFO `defer { ... }` rollback on `open-fridge` and multi-tile cleanup on `open-double-bed`, `repeat`/`while ... cancel_on_abort`, automatic `stack_obj` preservation across `for ... in objects_by_guid(...)` / `self.parts()` / `run_functional(...)`).

### 3. Deliberate v2 Improvements Over v0.3.0

While preserving all 20 object GUIDs, tile GUIDs, prices, display names, descriptions, sprite palettes, `SLOT` layouts, `FWAV` sound names/presets, and motive advertisements, the v2 migration makes four intentional improvements:
1. **Catalog category flags (`category:`):** All buyable objects now declare explicit Sims 1 catalog sort flags (`seating`, `surfaces`, `appliances`, `electronics`, `plumbing`, `lighting`, `misc`).
2. **Template-structured `BCON` layouts & auto-allocated private BHAV IDs:** Private `BCON #4096` entry ordering reflects shared template parameters and BHAV IDs are deterministically allocated without manual `@id(...)` pinning.
3. **`open-floor-lamp` dual graphics:** Adds a dimmed off-state graphic at index `1` (`OBJD.num_graphics = 2`), keeping index `0` as the lit state so initial `data[0]` remains `0` on boot.
4. **`open-cot` occupancy guard:** `"Take a Nap"` now checks `!self.occupied && sim.motive.energy < max_entry_energy` (matching `open-bench`, `open-dining-chair`, `open-sofa`, `open-shower`, and `open-toilet`) so a second Sim cannot claim an occupied cot.

### 4. Compiler Enhancement: Constant-Argument Helper Inlining

During migration, passing literal booleans/constants into `pub fn set_toggle_state(obj: Object, flag: &mut bool, enabled: bool, sfx: Sound)` revealed that unconditionally spilling immutable literal arguments into fresh BHAV locals (`let __arg_enabled_0 = 0; stack.attr[1] := __arg_enabled_0;`) obscured constant release writes (`:= 0`) from the static lock/toggle prover in `verify.rs` and wasted BHAV locals.

`crates/sims-antics/src/v2/lower.rs` now binds immutable, non-mutated/non-borrowed `i16` and `bool` literal/BCON arguments directly as `Binding::ConstI16 { air, value }` / `Binding::ConstBool(b)` during helper inlining, eliminating wasted local allocations and preserving exact constant-write visibility for `verify_object_safety` — while keeping `if` branch pruning restricted to true compile-time literals/`param`s (so dead branches inside helpers are still fully type-checked and validated) and preserving exact lexical shadowing precedence across `self`/`sim`/locals/params.

---

## Before / After Line-Count Comparison

| Component | Before (v1 `object.toml` + `behavior.antics`) | After (v2 `.antics`) | Delta |
|---|---:|---:|---:|
| Legacy `object.toml` files (20) | 1,014 lines | 0 lines (retired) | -1,014 |
| Legacy `behavior.antics` files (20) | 1,634 lines | 0 lines (retired) | -1,634 |
| Shared `common/helpers.antics` (1) | — | 94 lines | +94 |
| Shared `templates/*.antics` (4) | — | 113 lines | +113 |
| Migrated `objects/*.antics` (20) | — | 1,502 lines | +1,502 |
| **Total object source lines** | **2,648 lines (40 files)** | **1,709 lines (25 files)** | **-939 lines (-35.5%)** |
| World manifests (`pack.toml`, `surfaces.toml`, `starter-house.toml`) | 365 lines | 365 lines | 0 |
| **Total `open-pack/` lines** | **3,013 lines (43 files)** | **2,074 lines (28 files)** | **-939 lines (-31.2%)** |

---

## Verification

- **20 / 20** `open-pack/` objects pass `verify_pack_safety` with **0 violations**.
- **20 / 20** objects expand cleanly via `expand_v2_to_air` and `sims-antics expand <file> --pack open-pack` into valid `.air` that compiles via `compile_air` with matching BHAV counts, and are verified against explicit legacy GUID/name/`SLOT`/`FWAV` metadata expectations.
- **Full behavioral & regression suite green:** `open_pack_behavioral`, `open_pack_household_sim` (0 VM traps, all 8 motives, Cooking/Mechanical skills, bills, cancel safety), `open_objects_vm`, `v2_compiler` (including 2-Sim `open-cot` contention and constant-argument shadowing/diagnostic regressions), `cli_lift`, `install_roundtrip` (`28,690 / 28,690`), `structured_equiv` (`14,692 / 14,692`), and `sims-game --test cheats_cache_and_open_pack` (`7 / 7` both with and without `SIMS_INSTALL`).
- `unsafe_code = "forbid"`, zero new third-party crates, 100% deterministic `BTreeMap`/`BTreeSet` compilation, and zero commercial data written to disk or committed.
