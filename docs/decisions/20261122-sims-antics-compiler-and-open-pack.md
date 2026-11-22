# ADR: Clean-Room SimAntics Compiler (`sims-antics`) & Open Starter Pack Foundation

- **Date:** 2026-11-22
- **Status:** Accepted (Phase 1 implemented on `antics/compiler-and-iff-writer`)
- **Scope:** `crates/sims-antics`, binary IFF/FAR encoders, `.antics` DSL, `object.toml` manifests, procedural isometric art & synthesized WAV pipelines, in-memory completeness verification.

## 1. Context & Motivation

`sims-rs` already provides a deterministic SimAntics VM (`sims-vm`), clean-room binary readers (`sims-formats`), content indexing (`sims-content`), isometric rendering (`sims-render`), and interactive game shell (`sims-game`) capable of running a user-supplied copy of *The Sims: Complete Collection* (`--install`).

However, two major goals remain:
1. **Standalone Open Mode:** Allowing contributors, CI, and players without original commercial discs to launch `sims-game` using a 100% original, MIT-licensed **Open Starter Pack** compiled from human-readable source files.
2. **Modding & Clean-Room Tooling:** Providing a first-class, deterministic, zero-external-dependency SimAntics compiler and IFF/FAR packager so new objects, behaviors, sprites, and sound effects can be authored, version-controlled as plain UTF-8 text, and compiled to standard `.iff`/`.far` artifacts at build or load time.

## 2. Clean-Room Legal Boundary (Wire Format vs. Original Expression)

We enforce a strict separation between **interoperable binary wire formats** and **original creative expression**:

- **Wire format & execution semantics (functional interoperability):** Binary layouts for IFF 2.5 (`rsmp`), `BHAV`, `OBJD`, `OBJf`, `BCON`, `STR#`/`CTSS`/`TTAs`, `TTAB`, `SLOT`, `PALT`, `SPR2`, `DGRP`, `FWAV`, and `FAR!byAZ` archives are documented container/bytecode formats required to interoperate with `sims-formats` and `sims-vm`.
- **Zero proprietary assets in git:** **NO** original Maxis bytes, strings, BHAV trees, sprites, palettes, audio files, or lifted/decompiled `.antics` files derived from commercial installations may EVER be written to disk in the repository or committed to git. `scripts/check_repository.py` continues to enforce source-only, zero-binary, zero-blocked-payload hygiene across the full commit history.
- **Dedicated clean-room GUID namespace:** All original Open Starter Pack objects use the private clean-room GUID namespace `0x7F00_0001..=0x7F00_FFFF`, avoiding any collision with official or community GUID blocks.
- **In-memory completeness verification only:** Our `SIMS_INSTALL`-gated round-trip test (`crates/sims-antics/tests/install_roundtrip.rs`) reads commercial `.iff` objects strictly **in memory**, lifts chunks into `sims-antics` IR/text **in memory**, parses and re-encodes them **in memory**, asserts byte-for-byte parity **in memory**, and drops all buffers before exit. Nothing derived from `$SIMS_INSTALL` is ever written to disk.

## 3. Architecture of `crates/sims-antics`

`sims-antics` obeys all workspace determinism and safety rules (`unsafe_code = "forbid"`, `BTreeMap`/`BTreeSet` only, `libm` for transcendental math, zero new third-party dependencies).

### 3.1 Binary IFF & Chunk Encoders (`encode.rs`, `ir.rs`)
- Emits fresh Maxis IFF 2.5 containers with authentic version-0 `rsmp` resource maps preserving exact caller-supplied flags and distinct `(tag, id, variant)` entries while rejecting duplicates.
- Checked little-endian/bit-packed encoders for:
  - `BHAV` (formats `0x8000`, `0x8001`, `0x8002`: 12-byte header + 12-byte instructions, matching `sims-formats` and `sims-vm`; `0x8003` is explicitly rejected until end-to-end reader/VM semantics are added)
  - `OBJD` (versioned word arrays `136 | 138..=142`, minimum 84 bytes matching `ObjectDefinition::parse`, plus typed `ObjdSpec` builder emitting canonical 216-byte definitions)
  - `OBJf` (callback tables)
  - `BCON` (8-bit counted `i16` constant tables)
  - `STR#` / `CTSS` / `TTAs` (all formats: `0x0000` Pascal, `0xFFFF` Terminated, `0xFFFE` Commented, `0xFFFD` Localized, and classic big-endian `MacResource`)
  - `TTAB` (versions `4..=10`, both uncompressed and MSB-first bit-packed field compression)
  - `SLOT` (versions `4..=10`)
  - `PALT` (up to 256 RGB colors), `SPR2` (v1000 & v1001 RLE color/z-buffer/5-bit alpha frames), `DGRP` (versions `20000..=20004`), `FWAV` (sound event names)
  - `FAR!byAZ` version 1 archive writer validated directly against `sims_formats::far`

### 3.2 Two-Tier `.antics` Language (`lexer.rs`, `ast.rs`, `parser.rs`, `lower.rs`)
`.antics` unifies lossless low-level bytecode assembly and ergonomic high-level structured scripting in one language:

1. **Low-Level Exact Layer:**
   - Explicit `tree <id> ["label"] (format: 0x8002, kind: 0, args: N, locals: M, flags: F, revision: R) { ... }` headers.
   - Labeled nodes (`@label:`), exact raw instructions (`op 0xNNNN (b0, ..., b7) -> true: <target>, false: <target>;`), and readable mnemonics for core SimAntics primitives (`sleep`, `expr`, `find_best_object`, `animate`, `route`, `dialog`, `play_sound`, `set_motive_change`, `random`, `set_to_next`, `subroutine`/`call`, etc.).
   - Jump target validation catches unresolved labels, out-of-range node indices, and node counts exceeding the SimAntics 251-node limit (`0..=250`, reserving `253=error`, `254=true`, `255=false`) with `line:column` diagnostics.

2. **High-Level Structured Layer:**
   - Clean structured syntax: `tree` / `interaction`, `if` / `else if` / `else`, `while`, `return true` / `return false` / `return error`, `call`, `idle(ticks)`, motive/attribute/local/temp/param/global expressions (`:=`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`), comparisons (`<`, `<=`, `==`, `!=`, `>`, `>=`), `route_to(slot: N)`, `animate(...)`, `play_sound(...)`, `set_motive_change(...)`, `dialog(...)`.
   - Deterministically lowered into valid `BHAV` instruction graphs with automatic sleep parameter allocation when `idle(ticks)` is used.

### 3.3 Zero-Dependency `object.toml` Manifest (`manifest.rs`, `compile.rs`)
- Deterministic TOML subset parser (tables `[section]`, arrays of tables `[[interaction]]` / `[[slot]]` / `[[constant]]` / `[[sound]]` / `[[callback]]`, strings, integers including hex, floats, booleans, arrays, inline tables) using `BTreeMap` exclusively.
- Bundles object metadata, `.antics` scripts, procedural sprite specs, and synthesized audio presets into a self-contained `.iff` and multi-object `.far` pack.

### 3.4 Procedural Isometric Sprites & Synthesized Audio (`art.rs`, `audio.rs`)
- **Sprites (`art.rs`):** Deterministic geometric isometric furniture rasterizer producing all **4 rotations** (`1, 4, 16, 64` matching `sims-render` DGRP direction masks) × **3 zooms** (`1` far, `2` mid, `3` near) with 5-6-5 quantized shading, 5-bit edge alpha, and z-buffer depth values, encoded directly into `PALT` + `SPR2` + `DGRP` chunks.
- **Audio (`audio.rs`):** Deterministic 22050 Hz 16-bit mono PCM RIFF/WAVE synthesizer using `libm` oscillators, envelopes, and xorshift32 noise bursts for clean-room UI clicks, chimes, wood creaks, water/flush effects, and thuds.

## 4. Phase 2 Progress: On-Disk `open-pack/`, Multi-Tile OBJD & High-Level `.antics` Stress Suite

Implemented on `antics/open-pack-complex-objects`:
1. **On-Disk `open-pack/` Source Tree & Pack Pipeline:**
   - `open-pack/pack.toml` declares pack metadata, archive name (`open-pack.far`), and deterministic object list; each object lives in `open-pack/objects/<slug>/{object.toml, behavior.antics}` (100% plain UTF-8 text, zero committed binaries).
   - `compile_object_dir(dir)` and `compile_pack_dir(dir)` compile the full pack into `.iff` members, packaged `open-pack.far`, and a deterministic synthesized PCM WAV sound bank, rejecting cross-object GUID collisions or conflicting audio definitions.
   - `sims-antics` CLI (`crates/sims-antics/src/bin/sims-antics.rs`) provides `check <pack_dir>` and `build <pack_dir> --out <dir>` with zero third-party CLI crates.
2. **Multi-Tile `OBJD` Linking & `object.toml` Ergonomics:**
   - `[[tile]]` sections emit a catalog master `OBJD` (`sub_index = -1`, non-zero `master_id`) plus per-tile `OBJD` + `OBJf` chunks with `[dx, dy]` `sub_index` encoding, `lead` tile flag (word 63), `level` (word 56), and `interaction_group` redirection (`-1` lead / `+1` slave), matching `sims-vm`'s native multi-tile group factory.
   - `[[interaction]]` accepts `guard = "..."` (or `test = "..."`) for guard trees, and `[[string_table]]` / `.antics` `str` declarations emit `STR#` chunks (such as `STR# 301` for dialogs).
3. **High-Level `.antics` DSL Upgrades & Procedural Asset Expansion:**
   - Added named local variables (`let name = expr;` / `local name;`), `break;` / `continue;` inside `while` loops, named `bcon` constant blocks (`bcon.name`), named `str` table blocks (`str.name`), skill/data member aliases (`my.id`, `stack.person.cooking`, `stack.person.mechanical`, `my.prep_score`, `my.cook_score`), and first-class high-level primitives `create_object`, `remove_object`, `run_functional_tree`, `alter_budget`, and destination-capable `set_to_next`.
   - Expanded procedural isometric silhouettes (`art.rs`: `DoubleBed`, `Fridge`, `Counter`, `Stove`, `MealPlate`, `WallPhone`, `Shower` across 4 rotations × 3 zooms) and synthesized WAV presets (`audio.rs`: `PhoneRing`, `PhoneDial`, `Sizzle`, `ShowerSpray`, `WrenchClank`).
   - Battle-tested in `crates/sims-antics/tests/open_pack_behavioral.rs` against 10 clean-room objects (`0x7F00_0001..=0x7F00_0040`), including multi-tile double bed occupancy/cancel, 4-object Fridge→Counter→Stove→Meal Plate cooking chain, 2-way/3-way Wall Phone dialogs, and wear/broken/Mechanic-scaled Repair Shower.

## 5. Next Phase: Dual-Mode `sims-game`

In the next phase, `sims-game` will support **Dual Mode**:
- **Standalone Open Mode (default when `--install` is omitted):** Compiles `open-pack/` in memory via `sims-antics` and mounts it into `sims-content`/`sims-vm`/`sims-render` with zero external files required.
- **BYO Original Mode (`--install <path>`):** Preserves 100% existing Complete Collection fidelity and screenshot/simulation parity unchanged.
