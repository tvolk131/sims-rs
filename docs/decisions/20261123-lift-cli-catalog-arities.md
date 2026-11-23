# `sims-antics lift` resolves callee arities from the install catalog

- **Date:** 2026-11-23
- **Status:** Accepted
- **Scope:** `crates/sims-antics/` (`Cargo.toml` feature `install-catalog`, `arity_catalog.rs`, `structure_lift.rs`, `bin/sims-antics.rs`, `tests/cli_lift.rs`, `tests/structured_equiv.rs`, `tests/dependency_weight.rs`); `crates/sims-formats/src/behavior.rs` (`TreeScope`, `instruction_refused`, `Behavior::frame_dimensions`, shared with the VM); `crates/sims-content/src/index.rs` (`characters_mount`, non-blocking regular-file opens), `catalog.rs` (non-blocking scan); `crates/sims-vm/src/interpreter.rs` (uses the shared checks), `program.rs` (`load_library` export)

## Context

An inline subroutine call reads as many argument words as its callee declares, and a callee with more than 4 arguments reads them from the following instruction records. Structured lifting therefore refuses (`UnsupportedHeaderOrTarget`) any multi-instruction tree with an inline call whose callee arity is unknown.

The `structured_equiv` corpus run resolved callee arities the way `sims_vm::interpreter::behavior_from_owner` dispatches trees: globals (256..=4095) from `Global.iff`, private trees (4096..=8191) from the object IFF, and semi-globals (8192+) from the object's `GLOB`-linked IFF. The standalone CLI kept only private arities. As a result, `lift Objects.far --entry Fridges.iff` structured 43/51 trees, against 51/51 in the corpus (ADR `20261123-air-expansion-wave2`), and printed only "unsupported header or target" for the other 8.

## Threat model

- The install is not modified while a `lift` or `--verify-equiv` run is in progress.
- Supported hosts are macOS and Linux (x86_64 / aarch64), per product scope.
- Races from concurrent modification, other Unix targets, and Windows file identity are documented residuals (below), not handled.
- Inside that model, a catalog-derived arity is never wrong for the file lifted: the tool supplies one only after it has positively identified the file as exactly one entry of the catalog the VM would boot. Anything short of that leaves the range unknown, which only costs structure (labeled-CFG fallback), never correctness.

## Decision

**The rule: whenever the VM, booted the same way, executes this call instruction and copies its arguments, it copies exactly the argument count `lift` used.**

- A call the VM never executes in that boot cannot be misdescribed by a count: the link failed, the owner lost its GUID, or the caller chunk was discarded by the loader.
- A call that traps before any argument is copied cannot be misdescribed either. For example, an inline call to a six-argument callee at a tree's last node needs the following instruction records, finds none, and traps ("native call argument read past the end of the BHAV resource") before copying anything.
- Rejection at run time *after* the arguments are read does not change how many argument bytes were read. For example, a negative inline entry is refused only after the VM has read exactly the callee's count.
- "Booted the same way" means the base install with no neighborhood. Any `--neighborhood`-style option is out of scope.

The resolver asks the VM's own code (its boot path, catalog, library loader and dispatch checks) instead of restating its rules. Where a case is ambiguous the count is unknown, which costs structure (a labeled-CFG fallback), never correctness.

- **One resolver.** `sims_antics::arity_catalog::InstallArityCatalog` builds the `sims_content` install `Index` and `Catalog`, loads `Global.iff` arities, and resolves semi-globals through `Catalog::semi_globals`. `structured_equiv` and the CLI both call it. `sims-content` and `sims-vm` become optional dependencies of `sims-antics` (see *Dependency weight*).
- **Code-owner rules, by calling tree.** The VM finds a running tree with the same owner table it uses for callees (`behavior_from_owner`), so the *calling* tree's own ID says which file it came from and which owner it runs under. Arities are resolved per calling scope, not per file:
  - A `256..=4095` tree runs only from `Global.iff`, under the calling object: global callees are known, private and semi-global callees are owner-dependent.
  - A `4096..=8191` tree runs only as its own file's private tree, so that file is the code owner, and only if the file holds the **registered** definition of some object: an `OBJD` whose GUID no later registration replaces (`Catalog::by_guid`, the table `Program::link_with_pool` loads object files from). A file with no registered `OBJD` (none at all, or only GUIDs a later file takes over) never runs its private trees, so for them its private and semi-global callee ranges are unknown, and a `GLOB` chunk in it links nothing. The global range is still reported from `Global.iff`. That is harmless, because those trees never run. Private callees are its own, and semi-global callees come only from *its own* `GLOB` link. With no resolved link the semi-global range is unknown, because the VM has no semi-global library there and traps (`MissingTree`). This holds even when other objects link to the file as their semi-global: that role does not reach its private trees.
  - An `8192+` tree runs only as the semi-global library of a registered object whose `GLOB` links to its file, under that object. Semi-global callees are the file's own, and private callees are the caller's (unknown).
  - A tree outside those cases (a `256..=4095` tree outside `Global.iff`, or an `8192+` tree in a file no object links to) never runs. Its owner-dependent ranges are unknown.
  - The header shows one line when every lifted tree is in one scope (or the scopes agree), and otherwise `for trees 4096..=8191: … | for trees 8192+: …`.
  - These roles apply only to a positively identified file (see *Positive identification* below). Any other input has **no inferred role**: all three ranges are unknown, including its own private range, because the tool cannot tell whether the file is an object, `Global.iff` or a semi-global.
  - This is stricter than the self-fallback the corpus used before. It leaves the Complete Collection numbers unchanged: on real data the looser rule never changed an arity.
- **Duplicate IDs.** The VM's library loader keeps the last physical BHAV chunk of an ID, deleted ones included (`load_library_with_pool`). The original's loader skips deleted chunks.
  - Callees: the extractor reads every parseable chunk and leaves an ID unknown when its chunks disagree on arity, which is sound under either rule.
  - Callers: `sims_antics::arity_catalog::vm_kept_bhavs` picks, for every BHAV ID, the chunk the VM keeps (the last in file order, whatever its flags). An ID whose only chunks are deleted is included, because the VM loads it and can call it. `lift` lists, lifts and verifies exactly those chunks, and takes private `BHAV` labels for call comments from them. The `structured_equiv` corpus verifier checks the same chunks (on the Complete Collection the totals are unchanged). The file's arity context describes them. When several chunks share an ID, the output says so (`// duplicate BHAV IDs: 4096 (2 chunks), …; each shows the last chunk in file order, the one the VM keeps`). The chunks the VM discards never run, so no count can misdescribe them. Previously the CLI showed the first live chunk of a duplicated ID, which the VM discards.
- **Catalog built as the VM builds it.** `InstallArityCatalog::load` runs `Install::detect`, `Index::build` and `Catalog::build_with_pool` on the *unfiltered* index, exactly as the VM boots.
  - **Neighborhood `Characters`.** At boot the VM adds the neighborhood's `Characters` directory as a mount right after `GameData/UserObjects`, before `Downloads/` and the expansion mounts (`boot.rs` → `Index::add_characters`, at `Index::characters_mount`). A `Characters` file of the same name then takes precedence over a `GLOB` target in those later mounts. `lift` does not know which neighborhood the VM boots with, so a `GLOB` target in a mount at or after `characters_mount` gives an unknown semi-global range (`a neighborhood's Characters file named X would take precedence over Downloads/X, and the neighborhood is not known`). Targets in `GameData/Global`, `GameData/Objects` and `GameData/UserObjects` cannot be shadowed and stay known. This is the strict form: it does not look for an actual same-named `Characters` file. On the Complete Collection it changes no corpus number. Strictly, this goes beyond the rule, which covers only the base boot with no neighborhood; it is kept as the conservative choice because the game always boots with one.
  - A `Characters` file can also register an indexed object's GUID and take it over. That does not violate the rule: the rule's boot has no neighborhood, and even with one, the displaced file's calls would simply never run.
  - `GLOB` names resolve to the first file of that name in load order, even when it is a dangling symlink, as the Rust catalog does. Whether the original game does the same for duplicate names is unverified (ADR `20261113-later-guid-registrations-win-semi-globals`).
  - Identity matching is a side table built after the catalog and never changes it. If the catalog fails to build, no file gets an inferred role.
- **Libraries the VM would load.** Every library that supplies counts (`Global.iff`, the owner's own file for its private range, and a semi-global file) supplies them only if it is a regular file (checked with `stat` before any open, so a FIFO the catalog selects cannot block) and the VM's own library loader, `sims_vm::load_library`, accepts it. That loader is the one `Program::link_with_pool` uses, and it rejects a whole library over one bad chunk (e.g. a truncated BHAV that a per-chunk extractor would skip). Each count is also checked against the tree the loader keeps for that ID.
  - `Program::link_with_pool` fails the whole link when any library it loads fails. Without `Global.iff`, which every link loads, the VM does not run anything, so a `Global.iff` that is unreadable, not a regular file, or rejected by the loader leaves **every** range unknown: `none (the VM cannot link this install: …)`.
  - An owner or semi-global file the loader rejects leaves its own range unknown, with the reason (`the VM would not load …: …`). The global range is kept. That object's link fails, so none of its calls run, and under the rule no count can misdescribe them; the counts are still never taken from a library the VM rejects.
- **Runnable callee.** A count is supplied only for a callee whose argument count the VM reads on dispatch; a call the VM refuses earlier, or a callee whose frame it cannot size, gets none. The VM's checks now live in `sims_formats::behavior`, and both the interpreter and the resolver call them:
  - `TreeScope::of` (the ID ranges of `behavior_from_owner`);
  - `instruction_refused` (`step()` refuses an opcode with `0x8000` set, e.g. `0xA000`, before dispatch, so such an ID never gets a count even when a library has that tree);
  - `Behavior::frame_dimensions` (what `dimensions()` reads on call: an empty tree or a format other than `0x8000`/`0x8001`/`0x8002` traps before any argument is read, so it has no count).
- **Dependency weight.** The catalog sits behind the `install-catalog` cargo feature, on by default, so the CLI and the corpus test get it, while `--no-default-features` builds (compiler only) skip `sims-content`, `sims-vm` and, through it, `sims-net`, `iroh`, `tokio` and `redb`. Without the feature, `lift` gives every file `no inferred role: built without the install-catalog feature`, and `--no-catalog`, `--globals` and `--semiglobal` work as before. The alternative, moving `load_library_with_pool` into a non-networking crate, was not taken: the point is to call the VM's own loader, and a copy outside `sims-vm` would be a second loader to keep in step. `tests/dependency_weight.rs` runs `cargo tree --no-default-features` and fails if any of those crates appear.
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
  - `Equivalent` from `--verify-equiv` is relative to the callee arities used, and the header and the verify line say so. An explicit override, or a lot whose loaded callee versions differ from the base install's catalog, can supply a different arity, and then a different lift. A `Characters` file shadowing a `GLOB` target is excluded by the rule above; an override is not checked against anything.
  - `structure_lift` exposes `structured_lift_header_check` / `UnsupportedHeaderReason`, refactored out of the lifter's own guard so the two cannot drift. Fallback lines name the format, instruction count, kind-4 entry, out-of-bounds target, or the node and callee whose arity is above 4 or unknown. An unknown arity also names the missing source and the flag that would supply it.

## Results

- `structured_equiv` on the shared resolver (with the registered-owner, `Characters`, loader and runnable-callee rules): unchanged from ADR `20261123-air-expansion-wave2`. That is 27,781 / 28,690 structured and 909 labeled-CFG, with `0 / 0 / 0` compile failures, counterexamples and inconclusive results.

- `lift Objects.far --entry Fridges.iff`: **51/51 structured, 51/51 `Equivalent`** (strict and `--relax-padding`). Previously 43/51 + 8 labeled-CFG.
- `--no-catalog` reproduces the old 43/51. Each of the 8 fallbacks now reads, for example, `node 0 calls global tree 280 with inline operands but its arity is unknown; global arities not loaded (--no-catalog); pass --globals <Global.iff>`.

## Residuals

These are outside the threat model: they need the install modified during a run, or an unsupported host.

- `sims_content` still opens indexed files with blocking opens in `Resource::read` and the archive pool (`pool.rs`). The arity path `stat`s a library before those opens, so only a FIFO swapped in after that check makes them hang. They never misread.
- FAR member offsets are indexed from one archive. If the archive is swapped before the members are read, they are read at those offsets from the new archive (`index.rs` scan).
- The catalog reads an entry before the side table `stat`s it. A replacement in between can give the side table an identity whose contents the catalog never read (for example a different `GLOB` link).
- On Windows, the canonical-path identity does not see NTFS hard links, so a hard-linked alias under another role is not detected.
- Neighborhoods outside the install directory (for example a save store) are not consulted; the strict `Characters` rule treats every mount after `characters_mount` as shadowable, so none needs to be.
- On Unix targets without a known `O_NONBLOCK` value (outside macOS/BSD and Linux x86/x86_64/arm/aarch64/riscv64), the open blocks before `fstat` can refuse a FIFO.

## Declined

- **Missing `Global.iff`.** Review suggested keeping semi-global links when `Global.iff` is absent from the install. `Catalog::build_with_pool` requires it. Rebuilding the links without it would be a second resolver, which the task rules out, and the VM cannot boot such an install either. A `Global.iff` that is present but fails to load (or that the VM's loader rejects) likewise leaves every range unknown, since the VM cannot link.
