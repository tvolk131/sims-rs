# `sims-antics lift` resolves callee arities from the install catalog

- **Date:** 2026-11-23
- **Status:** Accepted
- **Scope:** `crates/sims-antics/` (`arity_catalog.rs`, `structure_lift.rs`, `bin/sims-antics.rs`, `tests/cli_lift.rs`, `tests/structured_equiv.rs`); `crates/sims-content/src/index.rs` (archive open guard)

## Context

An inline subroutine call reads as many argument words as its callee declares, and a callee with more than 4 arguments reads them from the following instruction records. Structured lifting therefore refuses (`UnsupportedHeaderOrTarget`) any multi-instruction tree with an inline call whose callee arity is unknown.

The `structured_equiv` corpus run resolved callee arities the way `sims_vm::interpreter::behavior_from_owner` dispatches trees: globals (256..=4095) from `Global.iff`, private trees (4096..=8191) from the object IFF, and semi-globals (8192+) from the object's `GLOB`-linked IFF. The standalone CLI kept only private arities. As a result, `lift Objects.far --entry Fridges.iff` structured 43/51 trees, against 51/51 in the corpus (ADR `20261123-air-expansion-wave2`), and printed only "unsupported header or target" for the other 8.

## Threat model

- The install is not modified while a `lift` or `--verify-equiv` run is in progress.
- Supported hosts are macOS and Linux (x86_64 / aarch64), per product scope.
- Races from concurrent modification, other Unix targets, and Windows file identity are documented residuals (below), not handled.

## Decision

- **One resolver.** `sims_antics::arity_catalog::InstallArityCatalog` builds the `sims_content` install `Index` and `Catalog`, loads `Global.iff` arities, and resolves semi-globals through `Catalog::semi_globals`. `structured_equiv` and the CLI both call it. `sims-content` moves from a dev-dependency to a dependency of `sims-antics`.
- **Code-owner rules.** The VM resolves private and semi-global callees through the *calling object*, not the file the tree lives in, so arities depend on the role of the lifted file:
  - An object IFF knows its private range (itself) and its semi-global range (its `GLOB` target). With no resolved link, the semi-global range stays unknown, because the VM has no semi-global library there and traps.
  - `Global.iff` knows only the global range. Its private and semi-global ranges are owner-dependent.
  - A semi-global IFF (some object links to it) knows its own semi-global range, since every owner that reaches it links to it. Its private range is owner-dependent. If the file also links to a different semi-global itself, its semi-global range is ambiguous and stays unknown.
  - A file under the install root but outside its index (a scratch copy) knows only its private range.
  - A file indexed more than once (hard links, or symlinks into two mounts) is resolved as each entry. If every entry gives the same arities and sources, that result stands. Otherwise its private and semi-global ranges stay unknown, and the header names the entries (`ambiguous: the same file is indexed as … and …`), since the owner depends on which entry the game loaded.
  - This is stricter than the self-fallback the corpus used before. It leaves the Complete Collection numbers unchanged: on real data the looser rule never changed an arity.
- **Duplicate IDs.** The VM's library loader keeps the last physical BHAV chunk of an ID, deleted ones included. The original's loader skips deleted chunks. The extractor therefore reads every parseable chunk and leaves an ID unknown when its chunks disagree on arity. That is sound under either rule.
- **Auto-discovery and file identity.**
  - `lift` walks up from the input path *as given* to the nearest install root (`Install::detect`), so a symlink into another install still uses the install it was reached from. Only after that does it try the canonical path.
  - The input is opened once. Its bytes are read from that handle, and its identity is taken from the same handle with `fstat`: `(device, inode)` on Unix. The handle stays open until the catalog lookup is done, so its inode cannot be freed and reused. At catalog load, every index entry's file is `stat`ed the same way. The input matches the entries with its identity (plus member name and offset inside a FAR), never by path.
  - This closes the path cases. `Objects/link/../X.iff` opens the `X.iff` beside the link's target, which can have a different `GLOB`, and a hard link or symlink can give one file two roles. It also closes an atomic save between the read and the lookup: the replaced file is a different inode, so it finds no entry and gets private arities only.
  - Off Unix, the identity is the canonical path, which cannot see a replacement.
  - Index paths are `stat`ed once, at catalog load. A path that is gone (`NotFound`, e.g. a broken symlink) or not a regular file cannot be the regular file being lifted, so it is left out. Any other failure, such as a directory without search permission, leaves a path that might be the lifted file under a role that cannot be resolved. While any such path exists, owner-dependent ranges are unknown for every lift, and the header names the paths.
  - A hidden test hook, `SIMS_ANTICS_TEST_REPLACE_INPUT_AFTER_READ=<staged file>`, renames a file over the input between the read and the lookup, so the atomic-save test runs through the CLI's own handle.
  - Only regular files enter the catalog, so a FIFO or socket in a mount such as `Downloads/` cannot block discovery. `sims_content` index building opens each `*.far` with `O_NONBLOCK` and refuses the handle unless `fstat` says it is a regular file, recording a diagnostic as it does for any unreadable archive.
  - Catalog errors name paths as `<install>/…`. A `Global.iff` that fails to load leaves only the global range unknown; semi-global links still resolve. On the Complete Collection this adds about 0.4 s.
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

## Residuals

These are outside the threat model: they need the install modified during a run, or an unsupported host.

- `sims_content` opens indexed files again later with blocking opens: `Resource::read`, the catalog scan (`catalog.rs`) and the archive pool (`pool.rs`). A FIFO that replaces a file between indexing and those opens makes them hang. They never misread.
- FAR member offsets are indexed from one archive. If the archive is swapped before the members are read, they are read at those offsets from the new archive (`index.rs` scan).
- Between the catalog's `stat` of an entry and its own read of that entry, a replacement can change what the catalog read (for example a `GLOB` link).
- On Windows, the canonical-path identity does not see NTFS hard links, so a hard-linked alias under another role is not detected.
- On Unix targets without a known `O_NONBLOCK` value (outside macOS/BSD and Linux x86/x86_64/arm/aarch64/riscv64), only the path pre-check guards against a FIFO, so a FIFO swapped in after that check blocks the open.

## Declined

- **Missing `Global.iff`.** Review suggested keeping semi-global links when `Global.iff` is absent from the install. `Catalog::build_with_pool` requires it. Rebuilding the links without it would be a second resolver, which the task rules out, and the VM cannot boot such an install either. A `Global.iff` that is present but fails to parse still degrades to "global unknown" only.
