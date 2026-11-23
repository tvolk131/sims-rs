# ADR: `.antics` v2 Language Ergonomics & Extensions (Tiers 1 + 2)

- **Date:** 2026-11-22
- **Status:** Accepted (PR A — Tiers 1 + 2 Language Ergonomics & `open-pack/` Migration)
- **Scope:** `crates/sims-antics/src/v2/`, `open-pack/`, `crates/sims-antics/tests/`
- **Related:**
  - `docs/decisions/20261122-antics-v2-language-and-air-ir.md`
  - `docs/decisions/20261122-open-pack-v2-migration.md`

---

## Context

The initial `.antics` v2 migration (`#393`) retired all 20 legacy `object.toml` files and reduced `open-pack/` object sources from **2,648 lines** (40 files) to **1,709 lines** (25 files), with zero `raw {}` blocks, zero `@id(...)` pins, and zero formal safety violations.

Reviewing all 25 v2 files in `open-pack/` revealed five remaining sources of repetitive boilerplate that lower cleanly onto the existing `.air` (Antics IR) layer without any IR or VM changes:
1. **Verbose slot, interaction, attribute, and sound defaults:** 8-line `slot { kind: routing, ... }` blocks, repeated `attenuation: 0.02`, `(min: -100)`, `attributes: 8;`, `play_sound(sfx, self);`, and trailing `return true;`.
2. **Repeated occupancy-lock pattern:** Nearly every interactive object duplicated `guard(sim) => !self.occupied && ...` alongside `with lock(self.occupied) { ... }`.
3. **Duplicated template interactions & unchanged parameter overrides:** `ToggleableDevice` consumers duplicated `"Turn On"` / `"Turn Off"` interactions, and template-inheriting objects re-listed unchanged default parameters.
4. **Out-parameter boilerplate in shared helpers:** Callers of `restore_motive_loop`, `restore_two_motives_loop`, `run_sleep_loop`, and `sit_comfort_loop` declared throwaway `let mut cycles: i16 = 0; ... &mut cycles` bindings and passed 7–9 positional arguments.
5. **Duplicated per-skill loops:** `open-bookshelf` duplicated `"Study Cooking"` and `"Study Mechanical"` because `sim.skill.*` had no compile-time `Skill` indexing equivalent to `sim.motive[motive]`.

---

## Decision

We extend the `.antics` v2 frontend (`crates/sims-antics/src/v2/{ast,parser,lower}.rs`) with five backwards-compatible Tier 1 + 2 features and migrate all 25 `open-pack/` `.antics` files to use them idiomatically:

### 1. Sane Defaults & Compact Shorthands
- **Compact `slot` declarations:** `slot <name> = routing([x, y, z], ...);` (defaults: `standing: true, sitting: false, ground: true, facing: north`) and `slot <name> = container([x, y, z], ...);` (defaults: `standing: false, sitting: false, ground: false, height: 5, facing: north`), with optional named overrides including `facing:`, `sitting:`, `standing:`, `ground:`, `min_dist:`, `max_dist:`, `optimal_dist:`, `proximity:`, `height:`, etc. The existing block form `slot <name> { ... }` remains 100% supported.
- **Interaction defaults:** `attenuation:` defaults to `0.02`; `advertises:` entries omit `(min: -100)` by default (`hunger +75`) while still supporting explicit `(min: ...)` / `(personality: ...)`; `run(sim)` implicitly returns `true` on normal fallthrough.
- **Auto-sized `attributes:`:** Omitting `attributes:` automatically infers `state_count.max(8)` (matching `sims-vm`'s minimum attribute table size), while an explicit `attributes: N;` override still takes effect (bounded below by `state_count`).
- **`play_sound(sfx)` default target:** Defaults receiver to `self` (`stack_obj` in interaction/functional trees, `false` in `init`/`main`), while preserving `play_sound(sfx, target)`.

### 2. Declarative `lock: self.<bool_state>` on `interaction`
- `interaction "Label" { lock: self.occupied, ... }` validates at compile time that `<bool_state>` is a declared `bool` state on `self`.
- Lowering automatically:
  1. Conjoins `!self.<bool_state>` into the interaction's guard (synthesizing `guard(sim) => !self.<bool_state>` if omitted, deduplicating a redundant leading `!self.<bool_state>`, or prepending `if self.<bool_state> { return false; }` on block-bodied guards), and
  2. Wraps `run(sim)` in `with lock(self.<bool_state>) { ... }`, so normal fallthrough, early `return false;`, and `cancel_on_abort` all release the lock and re-entrant `with lock(self.<bool_state>)` inside `run(sim)` is rejected at compile time.

### 3. Template `interaction` Inheritance, Hook Overrides, & Minimal Param Overrides
- `pub template` declarations may define `interaction "..." { ... }` blocks and default `fn` hooks (e.g. `fn on_power_changed(sim: Sim, on: bool) {}` in `ToggleableDevice` / `ToggleablePower`).
- Concrete objects inherit template interactions, override inherited `fn` hooks, override inherited interactions by label in-place, and only specify `param` assignments that differ from template defaults.
- `open-tv` inherits `"Turn On"` and `"Turn Off"` directly from `ToggleableDevice`; `open-floor-lamp` inherits from `ToggleablePower`, overrides `fn on_power_changed(sim: Sim, on: bool)` to switch between lit/off graphics and grant its `room_boost`, and preserves its exact `"Turn On Light"` (`autonomy: 40`, `room +40`) and `"Turn Off Light"` (`autonomy: 15`) interactions.

### 4. Value-Returning `fn -> i16` + Named / Default Parameters
- `fn` / `pub fn` now supports `-> i16` alongside `-> bool` and void `()`.
- Inline lowering binds the return value to a fresh caller local before running function-scoped `defer` / lock cleanups, supports discarding the return value when called as a statement, and enforces strict type checking (e.g., rejecting `-> i16` calls used directly as `if` boolean conditions).
- Helper parameters support default values (`ticks: i16 = 2, cap: i16 = 100`) and named call-site arguments (`restore_motive_loop(sim, Motive::Bladder, cycles: max_use_cycles, step: motive_step, ticks: use_ticks)`). Default parameter expressions evaluate hygienically in the callee's module/object scope so caller locals never shadow callee defaults.

### 5. Dynamic `Skill` Indexing (`sim.skill[skill]`)
- Adds compile-time `Skill` type (`Skill::Cooking`, `Skill::Charisma`, `Skill::Mechanical`, `Skill::Creativity`, `Skill::Body`, `Skill::Logic`) and `sim.skill[skill]` indexing lowering to `my.person[idx]`, mirroring `Motive` / `sim.motive[motive]`.
- `open-bookshelf.antics` consolidates `"Study Cooking"` and `"Study Mechanical"` through a single `fn study_skill(sim: Sim, skill: Skill)` helper.

---

## Example Before / After (`open-tv.antics`)

```antics
// Before (79 lines) -> After (45 lines)
use crate::common::helpers::{restore_motive_tracked, set_toggle_state};
use crate::templates::toggle::ToggleableDevice;

object "Open Retro Television" from ToggleableDevice {
    guid: 0x7F00_0052;
    price: 380;
    category: electronics;
    description: "A clean-room cabinet television with on/off power state and a cancel-safe Fun watch loop.";
    sprite: Sprite::procedural("television", primary: #765438, accent: #364856, trim: #423022);

    param watch_ticks: i16 = 2;
    param max_cycles: i16 = 4;
    param fun_step: i16 = 24;
    param max_entry_fun: i16 = 95;

    state watch_cycles: i16 = 0;

    sound switch_click = preset("ui_click", wav: "open_tv_click");
    sound jingle = preset("chime", wav: "open_tv_jingle");

    slot viewer = routing([0.0, -1.25, 0.0], sitting: true);

    interaction "Watch TV" {
        autonomy: 60,
        advertises: [fun +75],
        lock: self.occupied,
        guard(sim) => sim.motive.fun < max_entry_fun,
        run(sim) {
            if !self.power {
                set_toggle_state(&mut self.power, true, switch_click);
            }
            play_sound(jingle);
            restore_motive_tracked(
                sim,
                Motive::Fun,
                cycles: max_cycles,
                step: fun_step,
                completed: &mut self.watch_cycles,
                ticks: watch_ticks,
            );
        }
    }
}
```

---

## Line-Count Metrics (`open-pack/` `.antics` Files)

| Component | Legacy v1 (`object.toml` + `.antics`) | v2 Baseline (`#393`) | v2 Tiers 1 + 2 (PR A) | Delta vs Baseline |
|---|---:|---:|---:|---:|
| `common/helpers.antics` (1) | — | 94 | 142 | +48 |
| `templates/*.antics` (4) | — | 113 | 150 | +37 |
| `objects/*.antics` (20) | 2,648 (40 files) | 1,502 | 1,118 | -384 (-25.6%) |
| **Total `open-pack/` `.antics` (25 files)** | **2,648** | **1,709** | **1,410** | **-299 (-17.5%)** |

(Total cumulative reduction vs legacy v1: **-1,238 lines / -46.8%**.)

---

## Consequences

- **Positive:**
  - All 5 ergonomics features lower 100% cleanly onto existing `.air` with zero IR or VM modifications.
  - 100% backwards-compatible with existing v2 syntax (`slot { ... }`, explicit `attenuation:`, `(min: -100)`, `attributes:`, `play_sound(sfx, self)`, `with lock(...)`, positional calls, `return true;`).
  - All 20 `open-pack/` objects compile with **0 `raw {}`**, **0 `@id(...)`**, **0 safety violations**, and full behavioral/household/standalone game parity.
- **Constraints honored:**
  - `unsafe_code = "forbid"`, zero new dependencies, deterministic `BTreeMap`/`BTreeSet` compilation, strict PR A file scope (`crates/sims-antics/src/v2/**`, `open-pack/**`, tests, and decision doc only).
