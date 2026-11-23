# `sims-antics lift` resolves callee arities from the install catalog

- **Date:** 2026-11-23
- **Status:** Accepted
- **Scope:** `crates/sims-antics/` (`arity_catalog.rs`, `structure_lift.rs`, `bin/sims-antics.rs`, `tests/cli_lift.rs`, `tests/structured_equiv.rs`); `crates/sims-content/src/index.rs`, `catalog.rs` (non-blocking regular-file opens); `crates/sims-vm/src/program.rs` (`load_library` export)

## Context

An inline subroutine call reads as many argument words as its callee declares, and a callee with more than 4 arguments reads them from the following instruction records. Structured lifting therefore refuses (`UnsupportedHeaderOrTarget`) any multi-instruction tree with an inline call whose callee arity is unknown.

The `structured_equiv` corpus run resolved callee arities the way `sims_vm::interpreter::behavior_from_owner` dispatches trees: globals (256..=4095) from `Global.iff`, private trees (4096..=8191) from the object IFF, and semi-globals (8192+) from the object's `GLOB`-linked IFF. The standalone CLI kept only private arities. As a result, `lift Objects.far --entry Fridges.iff` structured 43/51 trees, against 51/51 in the corpus (ADR `20261123-air-expansion-wave2`), and printed only "unsupported header or target" for the other 8.

## Threat model

- The install is not modified while a `lift` or `--verify-equiv` run is in progress.
- Supported hosts are macOS and Linux (x86_64 / aarch64), per product scope.
- Races from concurrent modification, other Unix targets, and Windows file identity are documented residuals (below), not handled.
- Inside that model, a catalog-derived arity is never wrong for the file lifted: the tool supplies one only after it has positively identified the file as exactly one entry of the catalog the VM would boot. Anything short of that leaves the range unknown, which only costs structure (labeled-CFG fallback), never correctness.

## Decision

- **One resolver.** `sims_antics::arity_catalog::InstallArityCatalog` builds the `sims_content` install `Index` and `Catalog`, loads `Global.iff` arities, and resolves semi-globals through `Catalog::semi_globals`. `structured_equiv` and the CLI both call it. `sims-content` and `sims-vm` move from dev-dependencies to dependencies of `sims-antics`.
- **Code-owner rules, by calling tree.** The VM finds a running tree with the same owner table it uses for callees (`behavior_from_owner`), so the *calling* tree's own ID says which file it came from and which owner it runs under. Arities are resolved per calling scope, not per file:
  - A `256..=4095` tree runs only from `Global.iff`, under the calling object: global callees are known, private and semi-global callees are owner-dependent.
  - A `4096..=8191` tree runs only as its own file's private tree, so that file is the code owner. Private callees are its own, and semi-global callees come only from *its own* `GLOB` link. With no resolved link the semi-global range is unknown, because the VM has no semi-global library there and traps (`MissingTree`). This holds even when other objects link to the file as their semi-global: that role does not reach its private trees.
  - An `8192+` tree runs only as the semi-global library of an object linking to its file, under that object. Semi-global callees are the file's own, and private callees are the caller's (unknown).
  - A tree outside those cases (a `256..=4095` tree outside `Global.iff`, or an `8192+` tree in a file no object links to) never runs. Its owner-dependent ranges are unknown.
  - The header shows one line when every lifted tree is in one scope (or the scopes agree), and otherwise `for trees 4096..=8191: … | for trees 8192+: …`.
  - These roles apply only to a positively identified file (see *Positive identification* below). Any other input has **no inferred role**: all three ranges are unknown, including its own private range, because the tool cannot tell whether the file is an object, `Global.iff` or a semi-global.
  - This is stricter than the self-fallback the corpus used before. It leaves the Complete Collection numbers unchanged: on real data the looser rule never changed an arity.
- **Duplicate IDs.** The VM's library loader keeps the last physical BHAV chunk of an ID, deleted ones included. The original's loader skips deleted chunks. The extractor therefore reads every parseable chunk and leaves an ID unknown when its chunks disagree on arity. That is sound under either rule.
- **Catalog built as the VM builds it.** `InstallArityCatalog::load` runs `Install::detect`, `Index::build` and `Catalog::build_with_pool` on the *unfiltered* index, exactly as the VM boots. `GLOB` names therefore resolve to the first file of that name in load order, even when it is a dangling symlink, as the Rust catalog does. Whether the original game does the same for duplicate names is unverified (ADR `20261113-later-guid-registrations-win-semi-globals`). Identity matching is a side table built after the catalog and never changes it. If the catalog fails to build, no file gets an inferred role.
- **Libraries the VM would load.** A global or semi-global library supplies arities only if it is a regular file (checked with `stat` before any open, so a FIFO the catalog selects cannot block) and the VM's own library loader, `sims_vm::load_library`, accepts it. That loader is the one `Program::link_with_pool` uses, and it rejects a whole library over one bad chunk (e.g. a truncated BHAV that the per-chunk extractor would skip). Otherwise that range is unknown, and the header says why (`failed to read …: not a regular file`, `the VM would not load …: …`). The other ranges are kept.
- **Auto-discovery.** `lift` walks up from the input path *as given* to the nearest install root (`Install::detect`), then tries the canonical path. On the Complete Collection, loading the catalog adds about 0.4 s. Catalog errors name paths as `<install>/…`.
- **Positive identification.** Catalog-derived arities (global, semi-global, and the private range of an object) apply only when both hold:
  - the input's canonical path lies under the install root's canonical path, and
  - the input is exactly one indexed entry: the same `(device, inode)` for a loose file, or the same archive identity plus member name and offset for a FAR member.

  Otherwise the header reads `none (no inferred role: …)` for every range, says which rule applied (`no Sims install found above the input`, `file resolves outside the install root`, `file not in install index`, `the same file is indexed as … and …`, `… install index path(s) that could be this file could not be identified: …`, `install catalog failed to load`), and suggests `--no-catalog`, `--globals` and `--semiglobal`. A scratch copy, a file reached through a symlink into another install, and a file indexed under two roles all land here. This is a behaviour change: such files used to get their own private arities, and now need `--no-catalog` for them.
- **File identity.**
  - The input is opened once. Its bytes are read from that handle, and its identity is taken from the same handle with `fstat`: `(device, inode)` on Unix, the canonical path elsewhere. At catalog load, every index path is `stat`ed once and its identity derived from that one result.
  - This closes the path cases: `Objects/link/../X.iff` opens the `X.iff` beside the link's target, which can have a different `GLOB`, and a hard link or symlink can give one file two roles. It also closes an atomic save between the read and the lookup: the replacement is a different inode, finds no entry, and gets no inferred role. `catalog_lookup_never_matches_a_file_replaced_after_reading` tests that mechanism at the library level.
  - The input handle stays open until the catalog lookup is done, so its inode cannot be freed and reused for another indexed file. That lifetime is guaranteed by code structure (the handle is borrowed by every read and dropped at the end of `cmd_lift`), not by a test: APFS does not reuse inode numbers promptly, so no test on this host could observe the failure. There is no test hook in the binary.
  - An index path that is gone (`NotFound`, e.g. a broken symlink) or not a regular file cannot be the regular file being lifted, so it is left out of the side table. Any other `stat` failure, such as a directory without search permission, leaves a path that could be the lifted file. It blocks identification only for inputs of the same kind (a loose path for a loose input; a member of the same name for a member input) inside the same install, and the header names it.
  - Only regular files are opened. `sims_content` opens each `*.far` at indexing and each loose `*.iff` in the catalog scan with `O_NONBLOCK`, and refuses the handle unless `fstat` says it is a regular file, recording a diagnostic as for any unreadable file. A FIFO or socket in a mount such as `Downloads/` cannot block discovery. The scan's rejection does not remove the entry, so the arity path checks the file type again before reading `Global.iff` or a `GLOB` target (above).
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

- `sims_content` still opens indexed files with blocking opens in `Resource::read` and the archive pool (`pool.rs`). The arity path `stat`s a library before those opens, so only a FIFO swapped in after that check makes them hang. They never misread.
- FAR member offsets are indexed from one archive. If the archive is swapped before the members are read, they are read at those offsets from the new archive (`index.rs` scan).
- The catalog reads an entry before the side table `stat`s it. A replacement in between can give the side table an identity whose contents the catalog never read (for example a different `GLOB` link).
- On Windows, the canonical-path identity does not see NTFS hard links, so a hard-linked alias under another role is not detected.
- On Unix targets without a known `O_NONBLOCK` value (outside macOS/BSD and Linux x86/x86_64/arm/aarch64/riscv64), the open blocks before `fstat` can refuse a FIFO.

## Declined

- **Missing `Global.iff`.** Review suggested keeping semi-global links when `Global.iff` is absent from the install. `Catalog::build_with_pool` requires it. Rebuilding the links without it would be a second resolver, which the task rules out, and the VM cannot boot such an install either. A `Global.iff` that is present but fails to load (or that the VM's loader rejects) leaves only the global range unknown.
