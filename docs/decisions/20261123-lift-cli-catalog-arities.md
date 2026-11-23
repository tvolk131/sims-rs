# `sims-antics lift` resolves callee arities from the install catalog

- **Date:** 2026-11-23
- **Status:** Accepted
- **Scope:** `crates/sims-antics/` (`arity_catalog.rs`, `structure_lift.rs`, `bin/sims-antics.rs`, `tests/cli_lift.rs`, `tests/structured_equiv.rs`); `crates/sims-content/src/index.rs` (archive open guard)

## Context

An inline subroutine call reads as many argument words as its callee declares, and a callee with more than 4 arguments reads them from the following instruction records. Structured lifting therefore refuses (`UnsupportedHeaderOrTarget`) any multi-instruction tree with an inline call whose callee arity is unknown.

The `structured_equiv` corpus run resolved callee arities the way `sims_vm::interpreter::behavior_from_owner` dispatches trees: globals (256..=4095) from `Global.iff`, private trees (4096..=8191) from the object IFF, and semi-globals (8192+) from the object's `GLOB`-linked IFF. The standalone CLI kept only private arities. As a result, `lift Objects.far --entry Fridges.iff` structured 43/51 trees, against 51/51 in the corpus (ADR `20261123-air-expansion-wave2`), and printed only "unsupported header or target" for the other 8.

## Decision

- **One resolver.** `sims_antics::arity_catalog::InstallArityCatalog` builds the `sims_content` install `Index` and `Catalog`, loads `Global.iff` arities, and resolves semi-globals through `Catalog::semi_globals`. `structured_equiv` and the CLI both call it. `sims-content` moves from a dev-dependency to a dependency of `sims-antics`.
- **Code-owner rules.** The VM resolves private and semi-global callees through the *calling object*, not the file the tree lives in, so arities depend on the role of the lifted file:
  - An object IFF knows its private range (itself) and its semi-global range (its `GLOB` target). With no resolved link, the semi-global range stays unknown, because the VM has no semi-global library there and traps.
  - `Global.iff` knows only the global range. Its private and semi-global ranges are owner-dependent.
  - A semi-global IFF (some object links to it) knows its own semi-global range, since every owner that reaches it links to it. Its private range is owner-dependent. If the file also links to a different semi-global itself, its semi-global range is ambiguous and stays unknown.
  - A file under the install root but outside its index (a scratch copy) knows only its private range.
  - A path that matches more than one index entry only through symlinks (for example a file mounted both as an object and in the global directory) keeps its private and semi-global ranges unknown, since the owner depends on which entry the game loaded.
  - This is stricter than the self-fallback the corpus used before. It leaves the Complete Collection numbers unchanged: on real data the looser rule never changed an arity.
- **Duplicate IDs.** The VM's library loader keeps the last physical BHAV chunk of an ID, deleted ones included. The original's loader skips deleted chunks. The extractor therefore reads every parseable chunk and leaves an ID unknown when its chunks disagree on arity. That is sound under either rule.
- **Auto-discovery.** `lift` walks up from the input path *as given* to the nearest install root (`Install::detect`), so a symlink into another install still uses the install it was reached from. Only after that does it try the canonical path. It then matches the input to its index resource: exact path first, then the lexically normalized path (so `Objects/sub/../X.iff` is `Objects/X.iff`), then the canonical path. An exact or lexical match counts only when that entry is the file actually opened: if `Objects/link` is a symlink to `Objects/actual/sub`, then `Objects/link/../X.iff` opens `Objects/actual/X.iff`, which can have a different `GLOB`. When the opened file is not indexed, only its private range is known, and the header says why. It matches the loose file, or the FAR path plus member name and offset. Only regular files enter the catalog, so a FIFO or socket in a mount such as `Downloads/` cannot block discovery. `sims_content` index building likewise refuses to open a `*.far` that is not a regular file, and records a diagnostic, as it does for any unreadable archive. Catalog errors name paths as `<install>/…`. A `Global.iff` that fails to load leaves only the global range unknown; semi-global links still resolve. On the Complete Collection this adds about 0.4 s.
- **Overrides.**
  - `--globals <Global.iff | archive.far[!member.iff]>`: a FAR without `!member` defaults to `Global.iff`.
  - `--semiglobal <file.iff | archive.far!member.iff>`.
  - A spec that names an existing file is used as-is. Otherwise it splits at the first `.far!` whose prefix is an existing file, so directory names and member names may contain `!`.
  - Override members must match exactly: the full name, or the full name or last path component case-insensitively. Suffix matching is never used.
  - Each override replaces only its own ID range.
  - `--no-catalog` disables catalog lookup, leaving private arities from the file only. It is not byte-for-byte the old behaviour: a private ID whose chunks disagree on arity (e.g. a live 4-arg tree plus a deleted 6-arg duplicate) is now unknown, where the old path took one of them. On Fridges it reproduces the old 43/51.
  - All three are rejected with `--raw`, which does not use arities, and with an archive `--list` that has no `--entry`.
  - Arities are resolved after tree selection, so a bad override never hides a missing `--tree`.
- **Explainable output.**
  - Lift and `--list` output start with `// arity sources: global …; private …; semi-global …`. Each range shows its count and source: this file, install catalog (path relative to the install), explicit, or none, with the reason.
  - `Equivalent` from `--verify-equiv` is relative to the callee arities used, and the header and the verify line say so. An explicit override, or a lot whose loaded callee versions differ from the base install's catalog (for example a `Characters` or `Downloads` file taking precedence over a `GLOB` target), can supply a different arity, and then a different lift.
  - `structure_lift` exposes `structured_lift_header_check` / `UnsupportedHeaderReason`, refactored out of the lifter's own guard so the two cannot drift. Fallback lines name the format, instruction count, kind-4 entry, out-of-bounds target, or the node and callee whose arity is above 4 or unknown. An unknown arity also names the missing source and the flag that would supply it.

## Results

- `structured_equiv` on the shared resolver: unchanged from ADR `20261123-air-expansion-wave2`. That is 27,781 / 28,690 structured and 909 labeled-CFG, with `0 / 0 / 0` compile failures, counterexamples and inconclusive results.

- `lift Objects.far --entry Fridges.iff`: **51/51 structured, 51/51 `Equivalent`** (strict and `--relax-padding`). Previously 43/51 + 8 labeled-CFG.
- `--no-catalog` reproduces the old 43/51. Each of the 8 fallbacks now reads, for example, `node 0 calls global tree 280 with inline operands but its arity is unknown; global arities not loaded (--no-catalog); pass --globals <Global.iff>`.

## Declined

- **Missing `Global.iff`.** Review suggested keeping semi-global links when `Global.iff` is absent from the install. `Catalog::build_with_pool` requires it. Rebuilding the links without it would be a second resolver, which the task rules out, and the VM cannot boot such an install either. A `Global.iff` that is present but fails to parse still degrades to "global unknown" only.
