# Legacy mining audit (November 23, 2026)

This audit decides, crate by crate, whether anything in `legacy/crates/` is still left to
port. It applies the rule in `docs/decisions/20261123-legacy-crate-by-crate-cleanup.md`: a
legacy crate is deleted only when **nothing is left to mine**, and if in doubt it stays.
`docs/salvage-audit.md` was the starting list. It was taken at `36fdb157` (main), and the
audit was read-only, with no builds.

## Method

For every top-level module of every crate, the knowledge it holds was checked against the
current `crates/`:

- primitive semantics;
- native addresses and constants;
- file-format details;
- routing, render and sim rules;
- test fixtures and expected values.

Each item is classified as one of:

- **ported**, with its new location;
- **deliberately dropped**, citing `docs/vision.md`, an ADR or the salvage audit;
- **still to mine**.

A mechanical check backs the reading. Every native address (`0x4xxxxx`–`0x6xxxxx`, with or
without the `0x`) that appears in a legacy crate was matched against `crates/` and
`docs/`. Each cluster of uncited addresses was then traced to its legacy use and
classified. Most uncited addresses turned out to be dropped machinery:

- resume points of saved native call graphs;
- code-byte pins;
- vtables and captured-memory layout;
- Win32 and DirectDraw plumbing.

Tests that `include_str!` private fixture JSON can't be mined, because those fixtures were
never public. They are noted as such. Synthetic tests that encode real behavior count as
knowledge.

## Summary

| Crate | Lines | Verdict | Still to mine | Section |
| --- | --- | --- | --- | --- |
| sims-kernel | 5.5k | **removed** | 0 | [sims-kernel](#sims-kernel--removed) |
| sims-tools | 0.9k | **removed** | 0 | [sims-tools](#sims-tools--removed) |
| sims-fixtures | 6.8k | **removed** | 0 | [sims-fixtures](#sims-fixtures--removed) |
| sims-formats (leftovers) | 2.1k | **removed** | 0 | [sims-formats](#sims-formats-leftovers--removed) |
| sims-runtime | 129k | **removed** | 0 | [sims-runtime](#sims-runtime--removed) |
| sims-app | 88k | stays | 5 | [sims-app](#sims-app--stays) |
| sims-content | 98k | stays | 10 (3 ported) | [sims-content](#sims-content--stays) |
| sims-render | 55k | stays | 5 | [sims-render](#sims-render--stays) |
| sims-vm | 100k | stays | 3 (5 ported) | [sims-vm](#sims-vm--stays) |
| sims-sim | 187k | stays | ~20 | [sims-sim](#sims-sim--stays) |

Citations of the removed crates are written `legacy-final:crates/<crate>/...`. The tag
predates the move under `legacy/`, so a path `legacy/crates/<crate>/<path>` in today's tree
is `crates/<crate>/<path>` at the tag. Read it with `git show legacy-final:crates/<crate>/<path>`.
The removed crates are byte-identical at the tag and just before their deletion.

## Porting backlog

This is the consolidated list of what is still to mine. The per-crate sections below give
the details, the legacy line ranges and the evidence. Several items overlap across crates
(noted in brackets). A crate can be removed once its items are ported, or dropped with a
cited reason.

**sims-formats** (removed; both items done)
1. ~~Parity for an empty SKN texture line.~~ Settled from the disassembly: the reader (0x59a629) skips empty lines, so an empty texture line fails. `crates/sims-formats/src/avatar/text.rs` now does the same ([ADR](decisions/20261124-buy-catalog-famh-move-in-fuzzer-and-skn.md)).
2. ~~Re-express the catalog short-read tests and the synthetic text-avatar rejection tests.~~ They are in `crates/sims-formats/tests/integration/avatar_codecs/{catalog,avatar}.rs`.

**sims-runtime** (removed; both items done)
1. ~~Port the exploration fuzzer as the CI determinism and snapshot fuzzer.~~ The state layer is `crates/sims-sim/tests/integration/state_fuzzer.rs`; the tick-level driver is `crates/sims-vm/tests/integration/fuzz.rs`, which drives seeded `sims_net::protocol::Command`s over a live lot and compares a warm fork and several cold restores per tick ([ADR](decisions/20261125-vm-test-binary-travel-cache-tick-fuzzer-retire-sims-runtime.md)).
2. ~~Absorb the comparator skeleton from `fixture_replay.rs`, or record its invariants in an ADR.~~ The invariants and the `first_difference` contract are recorded in the [ADR](decisions/20261125-vm-test-binary-travel-cache-tick-fuzzer-retire-sims-runtime.md), citing `legacy-final:crates/sims-runtime/src/fixture_replay.rs`.

**sims-app**
1. Tutorial highlight marker (`0x5de13f`, `TutHigh.bmp`, placed at `y = ty − 53`) [also sims-content #10].
2. ~~Non-modal, anchored and large dialogs: dismiss box, hidden title for the tutorial controller, 1.2 aspect ratio, `BMP_` 5000–5024 picture icons.~~ Ported (#484): `sims_ui::dialog::Presentation` and the `MinimizeBox` dismiss box ([ADR](decisions/20261126-ui-buy-labels-family-export-dialog-presentation-currency.md)). Still open: the 40-second auto-answer timer (`widget_setting`), and the anchored placement is unverified against a capture.
3. Tutorial lesson table, for the tutorial port.
4. Magic Starter Kit collection: offers, package attributes, and the contents GUIDs.
5. Downgrade or port the two remaining PORT verdicts: `owned_worker.rs` and `bin/inspector.rs`.

**sims-content**
1. ~~Buy eligibility from the real TTAB (`0x457cb0`).~~ Ported to `crates/sims-content/src/buy_catalog.rs` ([ADR](decisions/20261124-buy-catalog-famh-move-in-fuzzer-and-skn.md)).
2. ~~Catalog-name fallback (`0x457d3b`, `*` + OBJD label).~~ Ported there too; the legacy CATS order and language fallback were wrong.
3. ~~Buy category labels (STR# 150, 200–207 and 210; catalog-6 remap; column order).~~ Ported there too.
4. Built-in avatar materials (`0x5d41db`).
5. ~~Footstep floor material (`0x5890a8`) [also sims-sim #2].~~ Ported (#485): `crates/sims-vm/src/footsteps.rs`, with floor categories from `floors.iff` in `LotPrices` ([ADR](decisions/20261126-walk-animator-states-and-footsteps.md)).
6. Censor appearance (`<profile>-needs` in `adult.cmx.bcf`, person word 30) [also sims-render #1].
7. Object graphic selection. It conflicts with the renderer today: out-of-range selectors wrap to 0, the DGRP view is picked by direction bitmask, and SPR# wins.
8. Notification text colours (`0x4ff31a`).
9. Numbered catalog open order (`Clothes.bcf` before `_17_Clothes.bcf`).
10. Tutorial highlight cell (low) [also sims-app #1].
11. Skinning FPU control word (note).
12. Resize deletion condition (`0x442866`, note).
13. Re-express the `tests/frame_strings.rs` test.

**sims-render**
1. ~~Censor mosaic (`0x4ac301`: block sizes, strict depth, global-RNG jitter, 555/565/556 packing).~~ Ported (#486): `crates/sims-render/src/censor.rs`, its rectangle from the posed `-needs` censor boxes, its draws from a per-tick copy of the global RNG ([ADR](decisions/20261126-render-censor-held-order-walls-glide.md)).
2. Smooth zoom and high-DPI design reference (`Camera::sprite_zoom`), or an ADR dropping it.
3. Object-preview fitting edge rules (signed facing division, measured-union centring).
4. Dialog object picture (120×120 preview or 45×45 icon). Check against an original screenshot.
5. Small checks: blit row-skip, the luminous sprite flag (`0x4`), and the skinning-order test.

**sims-vm**
1. ~~Community-lot RNG reseed on load (`0x46d09c..0x46d0e8`), with timer readings recorded as seeded inputs.~~ Ported: `sims_vm::boot::read_simulator`, `World::seed_from_host_clock`.
2. ~~Date indicator and indicator-notification queues (`0x4448a6`, `0x444962`, `0x4464db`, `0x4472a2`, `0x4477fb`).~~ Ported: `World::indicator_queues`, `selection.rs` `refresh_indicators`. One departure: a deleted person leaves every queue entry (`IndicatorQueues::forget`), where the original erases only the first and keeps a dangling pointer. The star pools are kept as the original keeps them (`IndicatorQueues::available_stars`, `stars_in_use`). Verified in the disassembly: made stars are appended in the order made (0x4464c9); a star let go from a head moves from in-use to the end of the available pool only if it was in use (0x4481eb: 0x4483bd); a refresh, an arrival and a deselection take the available pool's last star (0x447012, 0x447b63, 0x446ed9); a refresh and an arrival take it before the border-tile check, so a famous Sim on a border tile loses it (in neither pool), while a deselection checks first (0x446d17) and loses none; a star being removed leaves both pools before it is taken off the lot (0x44126f → 0x447d8b → 0x448097), so a fault in its holder's callback cannot leave it in use; the lot clear deletes the in-use pool, then the available pool (0x44602d). The arrival on a put runs when the old position was off the active lot by its size (0x433366) and the x word moved (0x4335a5). Saves from before format 11, and `WorldSnapshot` slots saved before the queues were kept, take their pools from where their stars stand (`queues_saved_before_kept`). Their order is an approximation: it follows the stars' registration order, while the original's pools follow the order the stars were let go, which can differ (a fourth star let go before the third). A `WorldSnapshot` save payload carries the pending refreshes after the world's encoding (`World::encode_snapshot`). Open: the Downtown Date arrival marker, since the port's travel writes the date link itself and the original's arrival sequence is unread; a star lost because the head slot's flags lack bit 2 (0x4470a8), since the port keeps no slot flags. Also open: the pools start empty on a house-file load, as the original's interface starts with empty pools, so any star marker such a file holds is in neither pool; whether the original's house file holds star markers is unread.
3. ~~Dialog icon mode 4, named icons (`0x530a7b`, `0x4368b6`).~~ Ported, all six categories: `sims_ui::dialog::named_icon`. `rel` takes the relationship panel's first person, whose last tie the original breaks by the place in its table of person types (0x4ea035, `Acquaintances::portrait_place`). Types made in play take places in the order made (0x4265fd → 0x4e8a3a); `Characters::made` keeps that order (`MadeTypes`). The stores that carry it have new versions that an older build refuses rather than saving the types back by GUID: the neighborhood store (format 5), the save records (format 3), the named snapshot records (record format 33) and the trip envelope (`TRVL` version 3). Older versions read back in GUID order. Open: a `rel` Sim with no portrait strip, which the original renders (0x4e94a4 → 0x4e8c5d); the port shows no icon.
4. The original sort with its exact tie order, used for object and wall draw order [also sims-sim #7]. **The sort is ported** (`sims_core::sort`). The wall draw order uses it (#486). Objects keep the depth order with holder→slot grouping (`0x437c56`): the legacy `rebuild_order` tile key was tried and not adopted, as it drew 6 ratchet pixels unlike the original ([ADR](decisions/20261126-render-censor-held-order-walls-glide.md)).
5. ~~Career text extras (alternate titles, per-level secondary strings).~~ Ported: the job panel (0x53074c) and `$Job` (0x44ca89) show the `STR#` ID + 1000 titles (0x4033d2); a list box of people (0x52ff7e) shows the first table's (0x4033a4). Both are kept.
6. ~~`cAverageHistory` (`0x4cb4a5`–`0x4cb635`).~~ Ported: `sims_ui::needs_history`. The consumer is identified, but the Needs bars' trend is not drawn yet.
7. Terrain-preparation gates (`0x49cf02..0x49d237`). Stays with the terrain presets (see below).
8. Parked for reference: `commander` and the scheduler storage, registry and visit model.

**sims-sim**
1. ~~Walk animator states 5, 7, 8 and 12, and the short sidestep.~~ Ported (#485, [ADR](decisions/20261126-walk-animator-states-and-footsteps.md)).
2. ~~Footstep and clip-sound selection.~~ Ported (#485): `crates/sims-vm/src/footsteps.rs`, `clip_events.rs`.
3. Route-cache invalidation on house refresh (`0x444805`/`0x46387c`) and the resume states.
4. "Go Here" ground click (`0x48cbec`, marker `0x7c4`).
5. Layer masking when posing.
6. The house's architecture value on the day turn: the load path computes it (`crates/sims-vm/src/lifecycle.rs:524-680`), but `daily.rs` leaves it stale, and the walls-only global-22 case (`0x4795c5`) fails. Also body scale from word 85 (`0x5a0932`).
7. ~~The original wall draw order (`0x5404b5`, `0x5409ab`, `0x53f1db`).~~ Ported (#486): run by run in the graph order (`0x540619`).
8. ~~Linear camera glide (`0x4c9a27`).~~ Ported (#486): 250 ms, single-precision slope, rounding half away from zero.
9. Floating text over a person (lifetime, styles, `sting_char_winner`).
10. ~~Currency formatter for negatives (`0x6007df`).~~ Ported (#484): `sims_ui::currency`, with the grouping table for every language.
11. ~~Zoom-out stops at 2 on house 99, and the ground-under-tile rule (`0x4af029`).~~ Ported (#486): `Zoom::zoom_out_on`; the ground under a level-1 floor is hidden only when the tile and its four neighbours are floored and all four neighbours are water (`0x4af029`). The legacy wording ("a neighbour is water" draws it) inverted the water test. The global enable byte `0x6c724a` is not ported.
12. The `move_objects` cheat at `0x6e4ad4`, refusal codes 0x25/0x26/0x27, and child visit order.
13. Calendar arithmetic (`0x473574`, `0x43520d`).
14. Build-tool classification (tool mode = group + 3).
15. Hand-tool hover flags (`0x486d95`).
16. Object resource-cache key (basename + extension, 31 bytes).

Low priority:

17. Portrait light direction.
18. Bone wiggle noise.
19. Content-path leftovers.
20. Grass generator for a fresh lot.


## sims-kernel — removed

**Verdict: DELETE-SAFE (removed in this change).** 5.5k lines with no game knowledge. Every
module is a Rust-port authority for Win32 named mutexes and anonymous events. Nothing in it
names a Sims.exe address, a game object or a file format: there are no `0x4xxxxx`–`0x6xxxxx`
addresses. Its hex literals are broker wire-protocol tags (`0x81`–`0x86`) and test values. No file outside `legacy/` cites
the crate.

| Legacy module/area | Knowledge | Status | Evidence |
| --- | --- | --- | --- |
| `lib.rs`, `server.rs`, `client.rs` (+`client/tests.rs`), `protocol.rs`, `endpoint.rs`, `auth.rs`, `unavailable.rs` | Unix-socket broker for named-mutex create/open/close; peer UID/audit-token auth; endpoint ancestry checks; v1 wire protocol | deliberately dropped | salvage-audit: "sims-kernel … DROP (Win32 mutex/event emulation)"; vision drops Win32 emulation |
| `event.rs` (+`event/tests.rs`) | Anonymous auto/manual-reset events with pulse and timeout semantics (host monotonic clock) | deliberately dropped | Same; the new sim is single-threaded and deterministic, with no host waits |
| `bin/sims-object-broker.rs`, `bin/sims-object-client.rs` | Foreground broker and exerciser CLIs | deliberately dropped | Same |

Legacy `sims-sim` and `sims-content` list `sims-kernel` in their `Cargo.toml`. They are not built, so nothing breaks; read the crate at the tag.

## sims-tools — removed

**Verdict: DELETE-SAFE (removed in this change).** 0.9k lines. The salvage audit said "Rebuild as a
thin CLI", and that has been done: `crates/sims-cli` (`census`, `install`, `houses`, `boot`) and
`crates/sims-antics` (`lift`, IFF/FAR inspection). The format facts it uses are all in
`crates/sims-formats`. No file outside `legacy/` cites the crate.

| Legacy module/area | Knowledge | Status | Evidence |
| --- | --- | --- | --- |
| `bin/sims.rs` `List`, `Inspect`, `Disassemble`, `Render` | Resource search, IFF object/DGRP/menu/label inspection, BHAV disassembly, drawgroup export | ported (superseded) | `crates/sims-antics/src/bin/sims-antics.rs` (`lift`, IFF/FAR inspection), `crates/sims-viewer`, `crates/sims-cli` |
| `bin/sims.rs` `Audit` | Every IFF and known chunk parser over the corpus | ported | `crates/sims-cli/src/census.rs` (corpus census, #18) |
| `save_cli.rs` | Inventory of houses and neighborhoods: chunk dispatch (`Arry`, `SIMI`, `HOUS` 0/1, `objt`, `ObjM`, `WALm`/`FLRm`, `FAMI`, `NBRS`, `NGBH`); the multipart flag read from OBJD bytes 20..22 and the GUID from 28..32 | ported | `crates/sims-cli/src/houses.rs`, `crates/sims-cli/src/census.rs:494-495`, `crates/sims-formats/src/{lot,saved_objects,neighborhood}.rs`, `crates/sims-content/src/catalog.rs` |
| `vm_cli.rs` (`Lifecycle`, `Inventory`, `Room`, `RoomResume`, `Deps`, `Chair`, `Resume`) | Laboratory runs of the legacy VM: the chair Sit scenario, single-object lifecycle, literal-call dependency walk, an opcode/operand census with an optional native dispatcher map | ported / dropped | The census is in `crates/sims-cli/src/census.rs`. The call graph is in `crates/sims-antics` (lift/verify). The lab scenarios drove the legacy VM, and the new VM's behavior is pinned by `crates/sims-vm/tests/` (e.g. `interaction_sweep.rs`, `boot.rs`). The native dispatcher map is oracle tooling (vision) |

## sims-fixtures — removed

**Verdict: DELETE-SAFE (removed in this change).** 6.8k lines. This is the private evidence store
(content-addressed blobs, case manifests, trace chunks, run reports) that legacy runners
used. It holds no game behavior, format facts or native addresses. Its hex constants are float
bit patterns in a round-trip storage test. No file outside `legacy/` cites the crate.

| Legacy module/area | Knowledge | Status | Evidence |
| --- | --- | --- | --- |
| `store.rs`, `archive.rs` (+tests), `model.rs`, `readset.rs`, `trace.rs`, `legacy.rs`, `bin/sims-fixtures.rs`, `README.md` | Store format v1 (`SIMSBL01` blob header, SHA-256 dedup, immutable case IDs), MessagePack typed payloads, trace chunks with keyframes and chained hashes, legacy JSON import | deliberately dropped | salvage-audit: "sims-fixtures … DROP (evidence store)"; vision drops evidence and receipt tooling. Oracle captures are now produced by the QEMU harness and compared by per-subsystem tests (e.g. `crates/sims-game/tests/goth.rs`) and the `TICK_43200` pin in `crates/sims-vm/tests/integration/boot.rs` |
| `tests/cli.rs`, `tests/storage_adversarial.rs` | Store CLI and adversarial storage tests | deliberately dropped | Same; they test the dropped store only |
| `examples/{publish_case,value_writer_bench,legacy_pilot}.rs` | Store demos and benches | deliberately dropped | Same |

The salvage audit marked its design as worth reading ("design only"). The README stays readable at the tag. The legacy `sims-runtime` (`fixture_replay`, `conformance`; itself removed since 2026-11-25) and `sims-render` (dev-dependency) named this crate; they are not built. The suite manifests in `legacy/fixtures/suites/` belong to the legacy `sims-runtime` conformance runner. That runner is now read at `legacy-final:crates/sims-runtime/src/conformance/`, so the manifests have no reader left and can go in a later cleanup.

## sims-formats (leftovers) — removed

**Verdict: DELETE-SAFE (removed 2026-11-24).** The 2.1k lines left here were the PE32 reader,
the native FAR mount reader and nine test files whose code now lives in `crates/sims-formats`.
The last two items are done: the empty SKN texture line is settled and the reader aligned
([ADR](decisions/20261124-buy-catalog-famh-move-in-fuzzer-and-skn.md)), and the two
synthetic test suites are re-expressed. The files are byte-identical at `legacy-final`, so
`legacy/crates/sims-formats/<path>` is read with
`git show legacy-final:crates/sims-formats/<path>`.

| Legacy module/area | Knowledge | Status | Evidence |
| --- | --- | --- | --- |
| `src/pe32.rs` (+`pe32/tests.rs`) | Bounded PE32/I386 reader for the executable's initial image (sections, imports) | deliberately dropped | salvage-audit (formats): "drop `far/mount.rs` and `pe32` (oracle tooling only)". No current crate reads Sims.exe bytes |
| `src/far/mount.rs`, `tests/far_mount.rs` | Native incremental FAR mount (0x5b9dbc): magic and version are not validated; the signed count is read at the manifest offset; a name of 2048 bytes or more fails ("native mount buffer"); a name is cut at its first NUL. The native catalog manager registers the archive and publishes each member as it is read, so a short read leaves earlier members visible and the archive registered, and a retry takes the already-mounted branch (see legacy `sims-sim/src/pose/clothing/catalog_mount.rs:121`) | deliberately dropped | salvage-audit (formats), as above. `crates/sims-formats/src/far.rs:83` validates more strictly (32 KiB cap, embedded NUL rejected). `crates/sims-content/src/index.rs` parses a whole directory before registering any member, so partial mounting is not reproduced. `crates/sims-formats/src/far.rs:83` parses names up to 32 KiB, and the content index then skips members whose names exceed 1,024 bytes (`crates/sims-content/src/index.rs:1108`), so the native 2,047-byte limit is never what decides. All of these differences only show on malformed archives. They are recorded here as a deliberate drop |
| `tests/resource_map.rs` `map_ordinal_order_overrides_file_and_numeric_id_order` | `rsmp` order decides resource order over file and ID order | ported | `crates/sims-formats/tests/integration/boundaries.rs` `a_resource_map_decides_lookup_as_well_as_order`, `a_resource_map_can_choose_the_earlier_chunk`, `only_version_2_5_headers_name_a_resource_map` |
| `tests/catalog.rs` (2 public tests) | Binary avatar catalog: a native short read on a count returns "none" and keeps earlier successes; the next count read then errors; three groups decode and leave a tail unread | ported | Behavior is in `crates/sims-formats/src/avatar/catalog.rs:60-62`. Tests: `crates/sims-formats/tests/integration/avatar_codecs/catalog.rs` `catalog_keeps_what_it_read_before_a_later_short_read`, `a_mixed_catalog_decodes_all_three_groups_and_leaves_the_tail_unread`. Its private-fixture motion test is not minable |
| `tests/skn_texture_names.rs` | **The original's SKN name reader skips empty lines.** With an empty texture line, the bone count (`1`) is taken as the texture and parsing then fails ("invalid avatar integer"). The legacy Rust reader, which follows it, fails on three EP7 accessory meshes | ported | Settled from the disassembly: 0x59a629 reads the name, texture and bone names with one getline loop (0x59a644, 0x59a6a9, 0x59a733) that skips empty lines (0x59a676, 0x59a6db, 0x59a769), and a failed read returns the failed stream (0x59a921). `crates/sims-formats/src/avatar/text.rs` `read_skn` now does the same. Tests: `skn_skips_empty_name_lines_so_an_empty_texture_line_fails` and, with `SIMS_INSTALL`, `installed_skns_fail_only_for_an_empty_texture_line` (3,165 SKNs, exactly 3 fail, all with an empty texture line) |
| `tests/text_avatar.rs` `text_avatar_rejects_invalid_or_unrepresented_data` | CMX `version 301`, nonzero trailing counts, NUL in names and trailing bytes are rejected; SKN out-of-range indices, NaN, oversize blends (32769), huge counts and incomplete lines are rejected | ported | `crates/sims-formats/src/avatar/text.rs` enforces these bounds. Test: `crates/sims-formats/tests/integration/avatar_codecs/avatar.rs` `text_avatars_reject_invalid_or_unrepresented_data`, on a synthetic SKN in place of the private fixture's. Its native-record comparisons are not minable |
| `tests/{career,neighborhood,surface_map,legacy_rle,resource_map(native),catalog(native motion),text_avatar(native)}.rs` | Native-reader parity on private captures | not minable (fixture JSON absent) | The `tests/fixtures/*.json` files were never public. The parsers are in `crates/sims-formats/src/{career,neighborhood,lot,legacy_sprite,iff,avatar}.rs` |

Nothing is left to mine. Legacy crates that name `sims-formats` in their `Cargo.toml` mean the workspace crate, not this one, and they are not built.

## sims-runtime — removed

**Verdict: DELETE-SAFE (removed 2026-11-25).** Both items that were left are done
([ADR](decisions/20261125-vm-test-binary-travel-cache-tick-fuzzer-retire-sims-runtime.md)): the exploration fuzzer is ported (state layer in
`crates/sims-sim/tests/integration/state_fuzzer.rs`, tick level in
`crates/sims-vm/tests/integration/fuzz.rs`), and the `fixture_replay` comparator's invariants
and `first_difference` contract are recorded in the ADR. The files are byte-identical at
`legacy-final`, so `legacy/crates/sims-runtime/<path>` is read with
`git show legacy-final:crates/sims-runtime/<path>`. Legacy `sims-app` still names the crate in
its `Cargo.toml` and sources; it is not built.

The audit below is kept as it was written, apart from the status of the two ported items.

Audited at `36fdb157` (origin/main). Read-only. Crate size: 129,029 lines. That is `src/original_session/` plus `original_session.rs` at 87.2k, `examples/` at 26.9k, `tests/` at 7.2k, `src/conformance/` at 5.8k, `exploration.rs` at 852, `fixture_replay*` at 977 and `bin/` at 85.

### 1. Verdict: **REMOVED** (the two small, self-contained items that were left are done)

Almost all of the crate is native call-graph continuation plumbing. That is what the vision drops ("resumable copies of native call graphs", captured memory, evidence tooling). Its algorithms live in the legacy `sims-sim::lot_presenter`, `sims-vm::scheduler::clock::draw`, `sims-vm::interface` and `sims-sim::object_catalog`. Those crates are audited separately. In this crate they are only sequenced against captured Goth "B" memory (tick 89073 and 88448/88449), with `pc`/return-address cursors, SHA-256 code pins and vtable checks.

Every address cluster sampled is one of four kinds:
- resumable `pc` or return-site lists;
- code-byte pins (`(addr, len, sha256)`);
- captured-memory global or vtable addresses;
- `compare_native` cut points.

Every function entry point with game behaviour is already cited in the current crates. Examples: Buy/move tool `0x4878f3/487105/487191/486960/486f5a/487e18` in `crates/sims-vm/src/buy.rs` and `boot.rs`; the draw frame `0x4d1b37/4d28df/4a8895/4aacb9` in `crates/sims-vm/src/frame.rs`; shadows and rows `0x434e23/4ae844/4ad8ac/4a9458` in `crates/sims-render`.

The crate held two items that were not yet mined when it was audited:
- the salvage audit's one **PORT** item, `exploration.rs` (the seeded command fuzzer comparing a warm fork with a cold restore, plus delta-debug minimisation). It is now ported in two layers: `crates/sims-sim/tests/integration/state_fuzzer.rs` and `crates/sims-vm/tests/integration/fuzz.rs`.
- the **REFERENCE** comparator skeleton in `fixture_replay.rs`. The oracle comparison is still in private comparison scripts outside the repository, so its invariants and `first_difference` contract are recorded in the [ADR](decisions/20261125-vm-test-binary-travel-cache-tick-fuzzer-retire-sims-runtime.md) instead.

Both were pure Rust with no native dependencies (anyhow, serde, sha2, serde_json) and had synthetic unit tests. With both done, the crate was DELETE-SAFE and is removed.

### 2. Module table

| Legacy module/area | Knowledge | Status | Evidence |
|---|---|---|---|
| `src/exploration.rs` (852, 9 synthetic tests) | The `Driver` trait: observe, candidates, execute, fork, checkpoint, restore. A seeded weighted SplitMix64 command choice with rejection sampling and no modulo bias. Two branches run side by side: a warm fork, plus cold `checkpoint→restore` at several command boundaries, deduplicated, with every one required. A per-tick trace is compared, reporting the first differing offset, so a transient divergence is caught even when command endpoints are equal. The input journal is written *before* execution. Panics are caught and reported. `verify_replay` excludes wall-clock data. `minimize` does delta-debugging. Outcomes: Passed, Incomplete or Failed. | **ported** (state layer: `crates/sims-sim/tests/integration/state_fuzzer.rs`, [ADR](decisions/20261124-buy-catalog-famh-move-in-fuzzer-and-skn.md); tick level: `crates/sims-vm/tests/integration/fuzz.rs`, [ADR](decisions/20261125-vm-test-binary-travel-cache-tick-fuzzer-retire-sims-runtime.md)) | salvage-audit:216-218 says PORT. Since 2026-11-24, `crates/sims-sim/tests/integration/state_fuzzer.rs` covers the state layer at command boundaries: seeded World commands with warm-fork, cold-restore and REDB-restore branches and per-command traces, record round-trips, and the REDB store against a model, with shrinking. Since 2026-11-25, `crates/sims-vm/tests/integration/fuzz.rs` covers the tick level: seeded co-op `Command`s over a live lot, a warm fork and several cold `Saved` restores compared per tick by `World::hash()` and the RNG streams, with panic capture and shrinking. Before it, `grep fuzz/cold.?restore/warm.?fork` over `crates/` found only perf "warm cache" benchmarks. Partial overlaps: `crates/sims-vm/tests/integration/save.rs` (one restore at tick 500, then 500 ticks compared by `World::hash`, with no commands); `crates/sims-vm/tests/integration/coop.rs` and `crates/sims-net/tests/integration/lockstep_memory.rs:734,852` (scripted commands, snapshot join, drift); `crates/sims-vm/tests/integration/interaction_sweep.rs` (exhaustive menu sweep with no restore comparison). Before the ports, none of them did randomised command choice, cold restore at arbitrary command boundaries, a per-tick first-offset trace or minimisation. |
| `src/fixture_replay.rs` + `fixture_replay/tests.rs` (977, 13 synthetic tests) | Comparator skeleton. `Driver{tick, observe, input, poll}` with `Poll::{CompletedTick, PartialPass, Boundary}`. Its invariants are listed below this table. | **recorded** (REFERENCE; invariants and `first_difference` in the [ADR](decisions/20261125-vm-test-binary-travel-cache-tick-fuzzer-retire-sims-runtime.md)) | salvage-audit:220-222. Today the oracle comparison is done by private scripts (private comparison scripts outside the repository; see ADR `20261114-comparison-harness-runs-again-capture-holds-1000.md`). `tools/harness/` only captures. No in-repo `first_difference` or boundary-invariant checker exists. Partial-pass semantics are ported (`crates/sims-vm/tests/interpreter/tick.rs:280`). Its `sims_fixtures` Store/CaseManifest/BlobRef plumbing is dropped (vision: evidence tooling). |
| `src/original_session.rs` `step` loop service order (1563-1820) | Host continuation order around one tick. Before the tick: UiEffect wait, removal graphic, selection assets (then HouseNotification), dialog construction, headline-catalog preparation (only with no modal), `update_contexts`. After it: the tick, a UiEffect re-check, republishing the selected person, dialog, the carried-object dependent graphic refresh, `host.service`, then the stop kind (Dialog, Budget or Faulted). | ported / superseded; rest deliberately dropped | The sim-observable order is documented in more detail in `crates/sims-vm/src/tick.rs:1-65`: lot RNG swap, scheduler pass, clock, then end-of-update dialogs, then removals, then the selection check, the signals and the hour/day turns. Frame-time effects are in `crates/sims-vm/src/frame.rs:1-50+`. UI effects, dialogs and answers go through `tick.rs` `Update.ui_effects` and `Player`. The remaining services (removal and selection graphics, headline catalog, dependent graphic) are presentation-asset continuations, which the vision drops. `tick.rs:41-62` lists them as "Not modeled". |
| `original_session/draw_parent/` (28.5k; 893 of the 1,160 unique addresses) | Resumable cursors over the native draw call graph: `4a92a2` post-window/model regions, `4aceb8` region loop, `4aee4c` rows, `4b0388/5dee3e` DirectDraw surface copy, `4b027d` EndFrame, `5ce76a` model gate, `4d28df` action window, `4a8bad` person visibility. Each child binds a SHA-pinned capture (`tick==89073`, `executes_target_code==false`) and calls a legacy `sims_sim::lot_presenter::*` algorithm (terrain_raster, walls, row_tail, floor_prep, sprite_gradient, object_shadow, room_masks…). | deliberately dropped | vision non-goals (call order, memory layout); salvage-audit:224 says DROP. A heuristic pass over the 1,160 address contexts: 785 are `pc`/return/cut lists, 84 are code pins or memory reads, 86 are bare pc lists, and the rest are vtables, string or global addresses, or `FUN_` labels. The algorithms are in legacy `sims-sim` (that crate's audit). Draw rules with sim effects are ported in `crates/sims-vm/src/frame.rs`. Render rules are ported in `crates/sims-render` (`scene.rs`, `shadows.rs`, `grass.rs`, `cutaway.rs`), which cite `0x434e23`, `0x4ae844`, `0x4ad8ac`, `0x4a9458`, `0x4aacb9`, `0x4a8895` and `0x4ab992`. The uncited entries (`4a92a2`, `4aceb8`, `4b0388`, `4b027d`, `5ce76a`) are dirty-region, DirectDraw or 3D-device plumbing. |
| `original_session/completed_*` (≈30k: simulation, lot, skin, skeletons, animations, prototypes, geometry, invocations, ordinary, assets, stock, tool_payloads, viewer, epoch, residents…) | Import of a captured native session. Code-byte pins (e.g. `completed_simulation.rs:200-300`, `(0x420c36,806,sha)`), vtable checks (`0x666514`, `0x6672b0`, `0x668d74`…), heap allocation sizes (`completed_tool_payloads.rs:10-23`, cTool factories `0x484520..0x4846fc`), preview and Move-group layout (`completed_stock.rs:164-224`), and the memory addresses of loaded tuning tables. | deliberately dropped (layout); tuning values ported | salvage-audit:224 says DROP; the vision drops memory mirroring. The motive-rate globals `0x6e4948..0x6e4a08` (`completed_simulation.rs:322-340`) have every field and derivation ported, read from tuning names, in `crates/sims-sim/src/motives.rs:50-130` (derived from 0x420c36). The room and lighting tuning `0x6e53e0..0x6e5424` (`completed_lot/environment.rs:62-80`) is ported in `crates/sims-sim/src/rooms.rs:1250-1340`. Movement tuning `0x6d079c..0x6d07e8` is ported in `crates/sims-vm/src/animator.rs:110-653`, which cites the addresses. Rig literal names (PELVIS, HeadFake, Palm…) are in `crates/sims-sim/src/head.rs` and `crates/sims-content/src/people.rs`. |
| Buy/tool UI: `move_tool/` (5.2k), `tool_selection*`, `purchase_cursor`, `current_tool`, `cursor`, `products`, `retained_tool_inputs/` | Native ordering of pickup, preview, commit, sale and undo/redo for the Buy move tool, the purchase cursor (`421986`/`4215e6`) and tool-selector callbacks. | ported | `crates/sims-vm/src/buy.rs` (mode switch `0x4cea5d/0x4ce44f`, history, cursor), `boot.rs:2266`, `crates/sims-game/src/game.rs:7195`, ADR `20261121-buy-mode-move-and-sell.md` (which gives the exact native order, including `0x4933dc` purge and `0x4cef9b→0x487191→0x487105`) and `20261121-buy-mode-catalog-placement-and-chair-replay.md`. The uncited `0x4cefce`, `0x476a35` and `0x485a19` are tool-receiver or refcount plumbing. |
| Input: `object_input`, `live_menu`, `ground`, `player_input/`, `neighborhood_input/` | Click → `48cbec/48c834/48dd67` pie-menu checks, the ordered same-tile fallback, and popup save/restore around return. | ported / dropped | `crates/sims-vm/src/orders.rs` (`0x48dd67`), `crates/sims-game/src/menu.rs` (`0x48c834`), ADRs `20261115-press-lists-pie-menu-again-reaching-entry-checks.md` and `20261115-pie-menu-listed-original-lists-listing-changes.md`. Neighbourhood replacement provenance is evidence tooling. |
| UI/publication: `camera_command`, `presenter/`, `publication/`, `property_refresh/`, `window_update/`, `window_parent`, `outer_parent`, `application_parent`, `interface`, `panel`, `transport/`, `notifications`, `ui_wait` | Continuations of `4ce390/4ce44f/4ce41b/53a01f/4764fe/4cee4a`: how notifications, camera and floor setters, and pause ordering are serviced. | ported (behaviour) / dropped (continuations) | `0x4ce44f` is in `crates/sims-render/src/scene.rs` and `crates/sims-game`. `0x4ce41b` is in `crates/sims-vm/src/primitives/special_effect.rs`. `0x53a01f` and `0x4764fe` are in `crates/sims-vm/src/selection.rs` and `boot.rs`. `0x4cee4a` is in `crates/sims-game/src/sound.rs`. Camera rotation as sim state is decided in vision §Multiplayer. |
| Timing: `clock.rs`, `completed_elapsed/`, `completed_epoch/` | The elapsed-frame caller and application ms. The effect-consumer qualification (`completed_epoch/effects.rs`) is evidence. | ported / dropped | Speed periods are in `crates/sims-sim/src/speed.rs` and tested in `crates/sims-net/tests/integration/lockstep_memory.rs:1166`. The frame schedule is in `crates/sims-vm/src/frame.rs` and ADR `20261118-goth-day-frame-schedule.md`. Wall-clock input is removed (vision §Simulation). |
| Sim-side calls: `valuation`, `postload`, `completion`, `controllers`, `object_budget`, `character_*`, `person_*` | `416534/416085` refresh and valuation, `43fc94` postload, `428041/44123f/440b2d` completion, `40726a` controllers, `5912fd` properties. | ported | `crates/sims-sim/src/rooms.rs`, `crates/sims-ui/src/house.rs` (416534/416085), `crates/sims-vm/src/visits.rs` and `lifecycle.rs` (43fc94), `crates/sims-sim/src/load.rs` (428041), `crates/sims-vm/src/person_factory.rs` (440b2d), `crates/sims-vm/src/boot.rs` (40726a), `crates/sims-sim/src/animator.rs` (5912fd). |
| Save/restore and evidence: `managed/`, `completed_replay/`, `service_capture/`, `completed_baseline/` | Version-2 owned-state restore, native replay traces, service-cut observers. | deliberately dropped / replaced | REDB saves are in `crates/sims-vm/src/boot/save` and `restore.rs`. ADR `20261113-state-hash-sha-256-postcard-encoding.md`. The vision drops evidence/receipt tooling. |
| `original_session/action_execution_tests.rs` and in-module tests | Synthetic tests of continuation guards, e.g. `real_session_step_forks_preserve_trace_save_rng_and_service_order`. | deliberately dropped | They test dropped plumbing. Fork determinism is the exploration item above. |
| `src/conformance/` (5.8k) + `src/bin/sims-conformance.rs` | Suites of private captured components (stacks, skin, skeletons, residents, prototypes, groups…). The `native_household` replay of the qualified Goth 88448→88449 tick, a subprocess runner with timeouts, memory admission and isolated workers. All are gated on `sims_fixtures` stores and pinned qualification hashes. | deliberately dropped | The vision drops evidence tooling and private-fixture tests. Of what they check, the component state is re-expressed by the current per-subsystem tests against captures (roadmap steps 3-4; for example `crates/sims-game/tests/goth.rs` replays compare the recorded words at updates 1-1200), and tick parity is covered by the `TICK_43200` pin in `crates/sims-vm/tests/integration/boot.rs`. Its only reusable piece, `first_difference`, is covered by the fixture_replay row. |
| `tests/` (26 files, 7.2k) | 22 files have `#[ignore]` (private Goth house or assets). `native_sprite_gradient.rs` and `retained_application_time.rs` are `#[cfg(feature="private-fixtures")]`, and their `tests/fixtures/*.json(.gz)` are **absent**, so they cannot be mined (and the functions they test live in legacy `sims-sim`/`sims-vm`). `conformance_cli.rs` tests runner orchestration. | deliberately dropped / not minable | The behaviours they touch are ported: the inventory primitive 0x33 (`crates/sims-vm/src/primitives/inventory.rs`), score curves (`crates/sims-vm/src/autonomy.rs`), the HEAD slot from post-constructor `0x45962b` (`crates/sims-vm/src/person_factory.rs:16`), valuation and refresh (above). |
| `examples/` (≈150 bins, 26.9k) | `completed_*_compare` diagnostics against private captures, timing benches, `probe_wine_save` (save probe), `retained_*` research importers. | deliberately dropped | Evidence tooling over private `cache/` inputs. The roughly 90 unique addresses they hold are pc/return lists (e.g. `completed_row42_compare.rs:94`). |
| `src/lib.rs` | Module wiring. | n/a | — |

`fixture_replay` invariants (referenced from the table row above):
- an input must not advance time or claim a completed tick;
- a partial pass must not advance time;
- a completed tick advances exactly 1;
- a service boundary must not advance time;
- poll, observation and tick limits apply;
- a reference tick gap is never turned into a wait;
- on a run that doesn't agree, the driver is returned for diagnosis when one was restored, with its observation at the stop when `observe` succeeds;
- `first_difference` is an RFC 6901 JSON-pointer walk that keeps missing, null and array distinctions apart;
- `Outcome` is Agreement, Diverged or Incomplete.

### 3. Backlog (done)

1. ~~**Port the exploration fuzzer.**~~ Done in two layers. The state layer is `crates/sims-sim/tests/integration/state_fuzzer.rs` ([ADR](decisions/20261124-buy-catalog-famh-move-in-fuzzer-and-skn.md)). The tick-level driver is `crates/sims-vm/tests/integration/fuzz.rs` ([ADR](decisions/20261125-vm-test-binary-travel-cache-tick-fuzzer-retire-sims-runtime.md)), with this mapping: `Command` = `sims_net::protocol::Command`; execute = `Lot::apply_coop_tick_batch`; observe = `World::hash()` plus the RNG streams; checkpoint/restore = `Lot::coop_snapshot_bytes` (a `Saved` encoding) → `Lot::restore_coop_snapshot`; fork = `Lot::fork_coop` → `Lot::restore_coop_fork` (an in-memory save and the co-op context as it is, refused where `Lot::coop_fork_blocker` says), because `Lot` is deliberately not `Clone`; the trace also carries the session's `StateFingerprint` and each command's `CoopCommandStatus`. It keeps weighted rejection sampling, several deduplicated cold-restore boundaries that are all required, the per-tick trace with first-offset reporting, the journal written before execution, panic capture and delta-debugging. Source: `legacy-final:crates/sims-runtime/src/exploration.rs`.
2. ~~**Absorb the comparator skeleton.**~~ Done by the minimal fallback: the invariants and the first-difference contract are recorded in the [ADR](decisions/20261125-vm-test-binary-travel-cache-tick-fuzzer-retire-sims-runtime.md), which cites `legacy-final:crates/sims-runtime/src/fixture_replay.rs:30-60` (Driver/Poll), `:219-397` (schedule run, poll invariants, compare), `:407-445` (`first_difference`) and the invariant tests in `fixture_replay/tests.rs:191-420`. An in-repo oracle comparator that replaces the private scripts can start from those.

Nothing else is left. The 1,160 unique addresses are continuation plumbing, code pins, or captured-memory and vtable layout. Their game behaviour is either ported (with cited addresses) or lives in legacy `sims-sim`/`sims-vm`, which those crates' own audits cover. With items 1 and 2 done, the crate was DELETE-SAFE and is removed.

Optional, not blocking: the addresses where the loaded tuning tables sit in memory (motives `0x6e4948-0x6e4a08` in `completed_simulation.rs:322-340`; rooms and lighting `0x6e53e0-0x6e5424` in `completed_lot/environment.rs:62-80`) are not cited anywhere current. The values are ported, but the addresses could be added as doc comments in `crates/sims-sim/src/motives.rs` and `rooms.rs` as `--watch` targets for the harness (read them at `legacy-final:crates/sims-runtime/src/original_session/`).

## sims-app — stays

Audited at `36fdb157` (worktree off origin/main). Read-only; no builds run.

### 1. Verdict: STAYS

Most of what the crate knows has been ported, and in most places the new code goes further than the legacy code did:
- The HUD cluster, strip, options-strip and catalog resource IDs and coordinates: every legacy HUD ID is present in `crates/sims-ui`.
- The mood gauge, motive meter and 565 palette.
- The bitmap-font metrics and line breaking.
- The message-dialog width search.
- The shortcuts, edge scroll and drag.
- The pick math.
- Sell and Undo lifetimes.
- Saves (now REDB).

Four pieces of original-game presentation and behavior are still unported, and nothing else in the repo records them:
1. The tutorial highlight marker's sprite and the `0x5de13f` placement formula. The new code draws a placeholder outline and marks it "unverified".
2. How non-modal, anchored and "large" message dialogs are presented. *Ported* (#484): `sims_ui::dialog::Presentation` and the dismiss box. The 40-second auto-answer timer is still open, and the anchored placement is unverified (backlog 2).
3. The tutorial lesson table: the order of lessons and what triggers each one. The tutorial itself is "not ported yet".
4. The Magic Starter Kit collection table, including GUIDs that appear nowhere in `crates/` or `docs/`.

Two PORT verdicts from `salvage-audit` were also never carried out and have no new equivalent: `owned_worker.rs` and `bin/inspector.rs`. Neither holds game knowledge, but the coordinator should either port them or downgrade them to DROP before the crate is deleted.

Everything else is one of:
- iced, host or recorder infrastructure;
- harness or receipt bookkeeping;
- a test behind private fixtures whose JSON is absent (`tests/fixtures/*.json` is not checked in), so it cannot be mined.

### 2. Module table

| Legacy module/area | Knowledge | Status | Evidence |
|---|---|---|---|
| `src/hud_cluster.rs` | Cluster resource IDs (2037, 2010, 2008, 2003, 2009, 2011–2014, 2023/2024, 4705–4707, pause 2026) and their 800×600 rectangles; the wide-window background last-column stretch (51b366/5160f0) | ported (refined) | `crates/sims-ui/src/hud.rs:37-130` (ids), `:870-1080` (rects; legacy rotate L/R swap corrected, clock box re-measured at 0x539ede), `:2990-3010` (strip stretch); `docs/decisions/20261114-live-control-panel-placed-measuring-original.md` |
| `src/original_hud.rs` | Mode-trim IDs and positions 4803–4806 (53a0b6); mood segments with f32 scale 0x3d6028c2; green/red strip crops; gauge-owner GUID resolution (4e986d→4e9753→4ea6a6); needs/subpanel button IDs 4500–4507/4510; options-strip IDs 4950–4952/4957–4959, separators 4916/4917; catalog category bitmaps 150–157 | ported | `hud.rs:835-836` (trim), `:2893-2926` (mood), `:1269-1275` (subpanel buttons, re-measured); `crates/sims-ui/src/household.rs:349` (GUID resolve); `crates/sims-ui/src/options.rs:28-50,222-223,683-688`; `crates/sims-ui/src/buy.rs:533` |
| `src/original_motives.rs` | Motive meter extent (5134bf), palette (5139dc), byte-quantised mix (5137e2), divider rule (51376e) | ported | `hud.rs:2928-2954` (cites legacy file) |
| `src/tutorial_font.rs` | FFN metrics (60b97f), coverage ×17 (60c446), 607d9e font choice | ported | `crates/sims-ui/src/font.rs:1-17` (cites legacy file) |
| `src/tutorial_dialog.rs` | Message-box skin (PopupInfoTiles 3008 9-slice, WinBtn 65×33) and line fit 60b620 | ported | `crates/sims-ui/src/dialog.rs:41-60`; `font.rs:204-260` |
| `src/tutorial_dialog.rs` | Width search 523fe3/524257 and the exclusion-band rounding 5351cc/534fe5 | ported | `dialog.rs:1410-1660` |
| `src/tutorial_dialog.rs` | Non-modal dismiss widget, `large` aspect target 1.2, picture-dialog extent, text colour | ported (#484), except the 40-second timer | `crates/sims-ui/src/dialog.rs` (`Presentation`, `MinimizeBox`); see backlog 2 |
| `src/tutorial_highlight.rs` (+ `original_viewer.rs:4447`, `ui_publication/view.rs:600-630`) | 0x5de13f marker origin; TutHigh 50 px frame; target table | **still to mine** | `crates/sims-game/src/game.rs:13045-13110` draws a 2 px yellow outline instead; `docs/decisions/20261121-ui-effect.md:15` says "unverified". See backlog 1 |
| `src/dialog_view.rs` | HUD bitmap loading, cell counts and magenta key | ported | `crates/sims-ui/src/art.rs` |
| `src/dialog_view.rs` | Placement of tutorial/anchored dialogs (observed top-right) and non-modal dialogs | **still to mine** | see backlog 2 |
| `src/support.rs` | Atomic temp+fsync+rename, output guard, letterbox `scene_cursor`, iced shader bridge, headless iced render, menu `entries` | superseded / dropped | Saves: REDB `crates/sims-game/src/autosave_worker.rs`, `docs/decisions/20261121-redb-migrations-and-autosave.md`. Pick: `crates/sims-game/src/main.rs` `canvas_point` (tested), `crates/sims-game/src/pick.rs`. Shader bridge is iced-only (salvage-audit: framework → winit+wgpu) |
| `src/original_viewer/owned_worker.rs` | Single-owner worker with capacity-1 channel and immutable publication | **PORT verdict not carried out**: architecture only, no game knowledge; sims-game runs the sim on the main thread | salvage-audit:230; backlog 5 |
| `src/bin/inspector.rs` | iced asset browser (sprite preview, interactions, behaviors, chunks) | **PORT verdict not carried out**: dev tool, no game knowledge; nothing equivalent in `sims-cli` or `sims-viewer` | salvage-audit:232; backlog 5 |
| `src/bin/room.rs`, `examples/dialog_frame.rs`, `src/laboratory.rs` (+tests), `src/original_viewer_play.rs`, `src/headless_input.rs` | iced entry point, authored-room demo (demo-only action whitelist), headless iced driver | dropped | salvage-audit:241-244 (iced framework). Laboratory tests are private-asset `#[ignore]` or iced pixel parity |
| `src/original_viewer.rs` (4.9k) | Host glue. Refs: 485a2e (move grab offsets), 4256f4, 46d553, 4ce81c, 4d1901→4764fe→46d9d6 poll order | ported / dropped | 485a2e: `crates/sims-render/src/cutaway.rs`, `crates/sims-game/src/game.rs`. 4764fe poll: `crates/sims-vm/src/boot.rs:1804`, `crates/sims-vm/src/selection.rs:27-60`. 46d553: `crates/sims-sim/src/world.rs`. The rest is the UI draining sim queues (salvage-audit finding 2, deliberately dropped) |
| `original_viewer/camera_shortcuts.rs`, `player_shortcuts.rs`, `viewport_drag.rs`, `exploration/camera_input.rs` | `,` `.` `=` `+` `-` PgUp/PgDn; Cmd-S/O; Space; right-drag; 20 px edge pan | ported (superset, more faithful) | `crates/sims-game/src/main.rs:3586-3690` (`classify_lot_hotkey`, 0x4d0ef0/0x4d12b1/0x51ba1c); edge scroll `crates/sims-game/src/game.rs:3275` (0x4e6246); right/left drag `main.rs:3038` |
| `original_viewer/save_feedback.rs`, `save_copy.rs` | Output-failure classification; copy-target aliasing via inode and CFStringCompare | superseded | REDB `SaveError` in `crates/sims-sim` autosave and `crates/sims-game/src/autosave_worker.rs`. Host policy, not original behavior |
| `original_viewer/{load_picker,load_feedback,recent_loads,feedback_notice,queue_cancel,resident_selection,neighborhood_usability,control_wiring,hud_cluster_publication,window_observation,window_identity,window_capture,widget_lifetimes,player_timing,strict_startup_probe(+tutorial_entry)}.rs` | Host ownership, revalidation, profiling, receipts | dropped | salvage-audit:239-240; vision.md non-goals (evidence/receipt tooling). Checked for game rules: only "no selected person hides needs", already ported at `hud.rs:1294` |
| `original_viewer/worker_host*` (4.4k incl. wiring tests) | Host worker wiring | dropped | salvage-audit:240 |
| `original_viewer/ui_publication*` (view.rs, journey probes) | HUD tables (see above) and tutorial camera-watch fields (`temps[0..1]` query) | ported / backlog 3 | view.rs:632-720 IDs and rects are in `hud.rs` and `options.rs`; the query semantics belong to the legacy `sims-runtime` `ui_wait` (other auditor; `legacy-final:crates/sims-runtime/src/original_session/ui_wait*`) |
| `original_viewer/exploration/**` (30k) | App-level input fuzzer and journeys. Embedded rules found: tutorial lesson table (`startup_play/host_journey/tutorial_camera.rs:18-200`); Magic Starter Kit collection (`startup_play/mystery_man*.rs`); 48bce9 sale detach (ported) | dropped, **except backlog 3 and 4** | salvage-audit:239. The fuzzer core is legacy `sims-runtime` (salvage-audit:216). Go-here/walking/object-interaction files (~7k) are harness bookkeeping only ("strict_original_parity_ticks":0, "native_qualification":false) |
| `src/window_session*`, `native/*.c,*.m,*.h` | Window-session recorder, macOS presentation and child-window C/ObjC | dropped | salvage-audit:239-240 |
| `scenarios/*.json` | Fuzzer configs (seed, limits, compare_fork, cold_restore_at) | dropped | Configs only, no game behavior. The fork and cold-restore idea is tracked under the `sims-runtime` fuzzer PORT (salvage-audit:216) |
| `src/original_viewer_*_tests.rs`, `original_hud_*_tests.rs`, `original_viewer/{tests,dialog_tests,visual_tests,pool_tests,appearance_tests,career_appearance_tests,dynamic_menu_tests}.rs` | Private-asset `#[ignore]` tests | not minable / ported | Mood-gauge corpus (890 + 144 cases) needs absent `tests/fixtures/hud-mood-gauge.json`; geometry ported. Sale test 48bce9/4933dc: `crates/sims-vm/src/buy.rs:1906-2110`, `crates/sims-game/tests/goth.rs:6080`. Outfit 590ca9/590cb8/58a178: `crates/sims-vm/src/outfits.rs` |
| `original_viewer_property_tests.rs` (36 public), `loaded_input_tests.rs` (20 public), `sha2_backend_tests.rs` (6 public), exploration `service_capture` / `host_journey` / `go_here` public tests | Proofs of frozen-surface and fresh-layout native call graphs, input-file identity, sha2 backend, harness invariants | dropped | vision.md non-goals (resumable copies of native call graphs, native mirroring). The room recount/light behavior they touch is covered by `crates/sims-vm/tests/integration/resize.rs` and roadmap step 22 |
| Address scan | the uncited-address list = 0x677180 (CPython ABI in `native/window_owned_child_python.c`). Bare-hex scan: 48bce9, 4ea6a6, 4efabc, 5137e2, 5160f0, 590ca9, 599469, 607d9e, 60b793, 4ce390, 4d1901 not cited in crates/docs | all covered by the rows above | 5de13f is cited in crates but its behavior is unported (backlog 1). The rest are call-site citations of ported behavior. 60b793 (zero-width line count = 1) is a trivial edge case |

### 3. Still to mine (backlog)

1. **Tutorial highlight marker (0x5de13f).**
   - **Sprite:** `CPanel/TutHigh.bmp`, a 150×50 atlas holding three 50×50 frames with a magenta key. The sprite loader is in `legacy/crates/sims-content/src/dialog.rs:11-36`.
   - **Placement:** `x = target.x + (target_w − 50)/2`, `y = target.y − 3 − 50` (`legacy/crates/sims-app/src/tutorial_highlight.rs:5-16`).
   - **Synthetic check:** the ZoomIn button at (26,522), 27×27, gives the marker origin (15,469) and the red-mask rectangle [26,474,53,504].
   - **Target table:** `ui_publication/view.rs:604-616`, which includes 4500 for the needs panel.
   - **Destination:** `sims-ui` hud (highlight item) and `sims-game::draw_ui_effect_highlights`, replacing the placeholder outline.

2. **Non-modal, anchored and large message-dialog presentation** (`tutorial_dialog.rs:87-106,284-395,511-545`; `dialog_view.rs:463-500`; also `legacy/crates/sims-vm/src/dialog/presentation.rs`). *Ported* (#484) into `sims_ui::dialog::Presentation` and the dismiss box, including `presentation.rs`'s `capture` and `choose_width` with the 1.2 target ([ADR](decisions/20261126-ui-buy-labels-family-export-dialog-presentation-currency.md)). Not mined: `widget_setting`, the 40-second auto-answer timer. The anchored placement is the legacy one, unverified against a capture:
   - **Dismiss widget:** a non-modal dialog shows a `Shared/Sys/MinimizeBox.bmp` widget (48×11, four 12×11 cells) at (w−bw−5, 5) instead of the bottom button.
   - **Modal extent:** `inset + body + 18 + 33 + 9 + 12`.
   - **Placement:** anchored dialogs (caller == owner) sit at the top right.
   - **Hidden title:** the title is hidden when the caller GUID is 0xc3249a1d (the tutorial controller).
   - **Large flag:** the `large` flag (`mode & 0x80`) uses an aspect target of 1.2 and a minimum width of max(image width, 130 when modal, 100 otherwise).
   - **Picture icons:** `BMP_` 5000–5024 flow text around image + 18.
   - **Destination:** `crates/sims-ui/src/dialog.rs` (`Presentation` since #484), plus `sims-game` input routing.

3. **Tutorial lesson table** (`exploration/startup_play/host_journey/tutorial_camera.rs:18-200`; Bob object 105 / neighbor 48 in `go_here.rs`):
   - **Order:** Zoom → Medium → Edge → Right → Rotate → Live → Bob → GoHere, then object interaction.
   - **Dialog origins:** Tutorial.iff BHAV 4116, instructions 1, 9, 11 and 20; BHAV 4125, instructions 1 and 3.
   - **Icons:** 5019, 5020, 5015, 5007, none, 5001.
   - **Recorded for each lesson:** the dialog operands and the presentation (non-modal, anchored, title hidden).
   - **What each lesson waits on:** Edge and Right wait on query [7,0] (scroll attempts), Rotate on [6,0]. The Bob → GoHere handoff keeps the same caller and stack object.
   - **Caveats:** these came from legacy-engine runs, not native ones. The tutorial itself is "not ported yet" (`crates/sims-game/src/game.rs:6353`).
   - **Destination:** a `sims-vm` or `sims-game` tutorial test and a roadmap note, cross-referenced with the legacy `sims-runtime` `ui_wait` (that crate's auditor; read it at `legacy-final:crates/sims-runtime/src/original_session/`).

4. **Magic Starter Kit collection** (`exploration/startup_play/mystery_man/collection.rs:5-12`, `mystery_man.rs:11-13`):
   - **Five offers** (tree → attribute, kind, GUID, count):
     - 4104 → attribute 4, kind 7, 0x99e81bec (MagiCoins), ×35
     - 4112 → attribute 0, kind 7, 0x1fec6005, ×1
     - 4124 → attribute 1, kind 8, 0x7ac1442e, ×1
     - 4125 → attribute 2, kind 8, 0x7aa53f96, ×1
     - 4126 → attribute 3, kind 8, 0x7bcb0f36, ×1
   - **Package attributes:** 0–4 are stock flags cleared as each item is collected; attribute 7 is "opened".
   - **Effect object:** 0xa79366fd is removed once the kit is empty.
   - **Contents GUIDs:** 0x0a87e8cf, 0x552a8297 and 0x8ec5750d appear nowhere in `crates/` or `docs/`.
   - **Caveat:** not native-qualified.
   - **Destination:** `crates/sims-vm/tests/integration/interaction_sweep.rs`, extending `goth_magic_hole_opens_kind13_special_panel_cleanly`.

5. **Decide the two unexecuted PORT verdicts** (no game knowledge):
   - `original_viewer/owned_worker.rs`: port it to `sims-game` or record that the main-thread sim supersedes it.
   - `src/bin/inspector.rs`: port it to `sims-cli` or as an egui dev tool, or downgrade it.

   Either way, update `docs/salvage-audit.md:229-232`.

## sims-content — stays

Audited 2026-11-23 against the worktree at `36fdb157`. This was a read-only audit: nothing was built and no repo file was edited. Scope:
- every top-level module in `src/` (98k lines, of which `native_bootstrap/` is 26.8k);
- `tests/` (83 files);
- `examples/` (39 files);
- the 54 unique native addresses in the uncited-address list;
- a re-scan of every `4xxxxx`/`5xxxxx` address written without `0x` outside `native_bootstrap`. Only 78 of these are absent from `crates/` and `docs/`, and each was checked.

### 1. Verdict: **STAYS**

Nearly all the load-bearing knowledge has been ported, and each item below cites where:
- the index and FAR reading;
- the OBJD GUID scan and catalog;
- house and neighborhood import;
- the linker core;
- careers and zoning;
- mount order;
- save-to-world order;
- house valuation;
- clothing and career outfit selection;
- surfaces;
- the job-popup atlas and portrait generators.

`native_bootstrap/`, `startup_memo`/`startup_inputs`, `owned_resources` and the `character_*`/`graphic_*` service layers are infrastructure that the vision drops. I checked their native addresses and tests; they hold no game data. One exception is noted under §3 item 11.

However, about ten pieces of real game behaviour still have their **only** written record in this crate. None of them appear in `crates/` or `docs/`, and one of them **contradicts** the current renderer:
- buy-catalog eligibility, name fallback and category labels;
- built-in avatar materials;
- graphic-selector and DGRP view rules;
- footstep floor material;
- censor appearance lookup;
- notification text colours.

Until these are ported, or written into `docs/`, the crate stays. The `legacy-final` tag keeps everything recoverable, but the bar is "nothing left to mine", and this crate does not meet it yet.

### 2. Module table

| Legacy module/area | Knowledge | Status | Evidence |
|---|---|---|---|
| `lib.rs` (`Index`, `Resource`, `read_archive`, `Document`) | Recursive scan; FAR manifest and directory read; 128 MiB member limit; 64 MiB directory limit; compressed FAR members rejected | ported | `crates/sims-content/src/index.rs:78-187,1043-1100` (adds native file-table order, mounts, `ArchivePool`); `crates/sims-formats/src/far.rs` (`MAX_MEMBER`, `MAX_DIRECTORY`) |
| `resource_scan.rs`, `resource_scan/object_guids.rs` (+ 4 test files) | OBJD discovery projection (GUID at payload +28, type +18, multipart +20); IFF 2.0/2.5 signature; 76-byte chunk header check | ported | `crates/sims-content/src/catalog.rs:1-120` (`Iff::scan_types`); `crates/sims-formats/src/iff.rs:374-383`. The bounded FD-cache handles and differential tests are infra (dropped) |
| `saves.rs` (`House::load`, `definitions`, `link_program`) | HouseNN/Neighborhood import; `Arry` 0/3/8/9/10 and 1/2 vs 11/12 by version; FLRm/WALm chunk 0 vs 1 at version 63; missing-neighbor family cleanup (0x424c10..0x424c56); neighborhood-scoped character GUIDs | ported | `crates/sims-content/src/neighborhood.rs:287-430` (adds `Gtab` translation); cleanup in `crates/sims-sim/src/neighbors.rs:281`; character mount in `crates/sims-content/src/index.rs:287-316`; later registration wins in `catalog.rs:38,308` (native 0x43ba52, as legacy `object_catalog/portraits.rs` notes). The legacy "reject non-identical duplicate providers" rule was prototype policy, superseded |
| `vm.rs` (`load_program`, animation tables, OBJD normalize, OBJf fallback, TTAB inheritance, person StringSets) | 30-offset OBJD callback fallback; 0x431759 normalize; STR# 150-160 native tables (0x59d860); TTAB local then semiglobal and 0x45726a master TTAB; skin tones STR# 8192; BodyStrings via 0x587b4b; FWAV names; legacy-TTAB skip | ported | `crates/sims-vm/src/program.rs:211-216,724-780,1068-1110,1793-1823`; `crates/sims-vm/src/animator.rs:591-604`; `crates/sims-content/src/people.rs:186-200`; `crates/sims-content/src/family_import/outfits.rs:7`. The two-object lab fixtures (`chair_room` etc.) are authored scaffolding (dropped) |
| `careers.rs` | `work.iff` CARR in resource order; `Careers.txt` override | ported | `crates/sims-vm/src/program.rs:228-240,1315,1439,1550-1580`. The localization kernel is in legacy `sims-vm/career_text.rs`, which is outside this crate |
| `zoning.rs` | LotZoning STR#1; Magic Town 90-98 tail (bit 0x80) | ported | `crates/sims-content/src/neighborhood.rs:164-240` |
| `compatibility.rs` | Corpus opcode census | ported (core) | `crates/sims-cli/src/census.rs`. The legacy extras (operand-variant, expression-byte and semi-global call-resolution histograms) are tooling, not game rules |
| `avatar_installation.rs` (+`objects.rs`), `avatar_mount.rs`, `avatar_mount_tests.rs`, `avatar_catalog.rs` (+`startup/`) | Mount order (0x43ae81); avatar search paths (0x5835ed); texture paths (0x414633); download providers (0x43bdc5); file-table hash order; loose file over archive member; cmx before bcf before ndx | ported | Compared line by line with legacy `sims-sim catalog_paths::initial_mounts`: `crates/sims-content/src/import_wardrobe.rs:23-60,740-797`, `people.rs:1076-1082`, `install.rs:121-148`, `file_table.rs`, `tests/install.rs`. Lines 56-77 of `avatar_installation.rs` are only this project's cabinet-extraction bindings (the salvage audit calls for that to be REWRITTEN; done by `Install::detect`). **Gap:** numbered-catalog open order (§3 item 9) |
| `object_catalog.rs`, `object_catalog/{household,portraits,saved_types,retained/*}` | Registration order; `filter_flagged`; type attributes (word 90, reset-per-lot bit word 91&2); global controllers (type 7/12, word 42 ≠ 0); latest-registration GUID lookup | ported | `crates/sims-vm/src/program.rs:1194-1225`; `crates/sims-vm/src/load.rs:350-380`; `crates/sims-content/src/catalog.rs`. `retained/*` and `saved_types` are retention/checkpoint infra (dropped, vision) |
| `object_catalog/products.rs`, `products/labels.rs` | Buy membership; CATS before CTSS; name fallback; TTAB eligibility (0x457cb0); category labels UIText STR#150/200-207/210 | ported | Sort and sort-word eligibility in `crates/sims-ui/src/buy.rs:690-790,1584-1630`; §3 items 1-3 in `crates/sims-content/src/buy_catalog.rs` (`Catalog::buy_entries`, `CatalogText`, `BuyLabels`). `sims-ui` does not use them yet |
| `object_catalog/thumbnail.rs` | 0x4ccaa0 composition (halve width over 45, centre with truncation) | ported | `crates/sims-ui/src/thumbnail.rs:193-200` |
| `save_storage.rs` | Save-to-world order: SIMI/context before controllers; action serial raised by 0x41f1c5; NBRS defaults (0x430ad6); family word rebuilt from FAMI (0x425244); 0x43fe0a bit7; secondary append order (0x4228a9); room pass before function 7 (0x442659); calendar statistics order; TATT overlay (0x43a516) | ported | Sampled about 20 addresses, all present: `crates/sims-vm/src/load.rs`, `objects.rs:473-486`, `crates/sims-sim/src/neighbors.rs:71-73,281,376`, `crates/sims-sim/src/world.rs:1114-1128`, `crates/sims-vm/src/daily.rs:209-216` |
| `house_loading.rs` | Post-load visit order (0x415eff F0, then 0x442659, 0x428041/0x428679, 0x43fc94 visits); person tail (0x5893a9, 0x459d23) | ported | `crates/sims-vm/src/visits.rs`, `load.rs`, `removal.rs`; `crates/sims-sim/src/load.rs`. Every address checked is cited |
| `house_startup.rs` (+`saved_arrays/`) | Resize at 0x415e20 (shrinking, off-lot profile); saved-array continuations | ported (profile) / dropped (continuations) | `crates/sims-vm/src/load.rs:84-130`. Continuations are resumable copies of a native call graph (vision). Note-level gap: §3 item 12 |
| `house_valuation.rs` | Build.iff STR#129/130/131/133/134/135 prices; built-in floor ids 1..29 and wall ids ≤30 or 248..255; CRT `atoi` | ported | `crates/sims-ui/src/house.rs:465-486,646-700`; `crates/sims-content/src/neighborhood.rs` `leading_number` |
| `surfaces.rs` (+`surfaces/resource_names_tests.rs`) | Floor/wall SPR frames (S/M/L; 4 wall faces); object wall masks (OBJD+0xa2); split floor 255 remap (0x4a0e3b) | ported | `crates/sims-content/src/surfaces.rs`; `crates/sims-render/src/assets.rs:972-985`; `crates/sims-sim/src/load.rs:177,717-745` |
| `household.rs`, `domestic.rs`, `stairs.rs` | Authored lots, funds and motives for acceptance tests | dropped | Their own docs say "authored scenario inputs". Real house loading replaces them (salvage-audit: sims-content DROP fixtures; vision) |
| `catalog.rs`, `assets.rs` (+`animation_catalogs`, tests) | Controller linking; animation-clip discovery | ported / superseded | `crates/sims-vm/src/load.rs:350-380`; `crates/sims-content/src/animations.rs` (0x59dc32) |
| `neighborhood_input.rs` | Installed-house list | ported (core) | `crates/sims-content/src/neighborhood.rs:251-268`. The legacy alias, overflow and `+1` rejection is self-described "proposed, not observation evidence" (prototype policy) |
| `avatar.rs`, `saved_avatar.rs`, `skeleton_resources.rs`, `saved_layers.rs` | Preview outfit; `texture_override` (0x599e87); skeleton catalog | ported / infra | `crates/sims-people/src/appearance.rs:166-174`; `crates/sims-vm/src/objects.rs:705,744` |
| `avatar_material.rs` | Built-in material colours (0x5d41db) | **still to mine** | Not in `crates/` (§3 item 4) |
| `body_profile.rs` | BodyStrings padded to 35 (0x587b4b) | ported | `crates/sims-content/src/family_import/outfits.rs:7` |
| `clothing_catalog.rs`, `clothing_profile.rs` | Clothing enumeration; career outfit (0x45ee5f); pajamas (0x45f034); migrations (0x409315) | ported | `crates/sims-vm/src/outfits.rs:1-11,603-775`; `crates/sims-people/src/suit.rs:2-35,337-379`; `crates/sims-content/src/wardrobe.rs`, `import_wardrobe.rs` |
| `character_appearance.rs`, `character_model.rs`, `character_portrait.rs`, `character_geometry.rs` | Appearance re-read (0x5888d3); posture clip tables; portrait draw | ported / infra | `crates/sims-vm/src/frame.rs:43-53`, `animator.rs:1211-1260`; `crates/sims-people/src/caf_preview.rs`, `pie_head.rs`. The FPU word 0x037f/0x003f for skinning is a presentation detail (§3 item 11) |
| `character_initialization.rs`, `character_censor.rs` | Censor appearance `<profile>-needs` inside `adult.cmx.bcf` | **still to mine** | §3 item 6 |
| `character_loading.rs`, `character_loading/` (incl. `floors.rs`, `live/`) | Footstep floor material (0x5890a8 / 0x49fd95); live person construction services | floors: ported (#485); rest infra | §3 item 5; `crates/sims-vm/src/footsteps.rs` |
| `graphic_{initialization,loading,materials,profiles,retirement,update}.rs` (+dirs) | Ordinary 3D animator services (plumb bob) | dropped (infra) | salvage-audit: sims-content DROP `graphic_*`. Bob in `crates/sims-people/src/bob.rs`; own-graphics CRT draws in `crates/sims-vm/src/factory.rs:398-410` |
| `object_graphics.rs` | Graphic selector wrap (0x438954); DGRP view by direction-bit and exact zoom (0x407847); SPR# before SPR2 (0x4c8e0d); id interval (0x45768a) | **still to mine (conflicts with current code)** | §3 item 7 |
| `object_resources.rs` (+`overlay`, `owner/`) | Mounted IFF resource owner | dropped (infra) | salvage-audit DROP list |
| `autonomy_resources.rs` (+dir) | FCNS2 observed-profile service | dropped (infra) | FCNS constants in `crates/sims-vm/src/program.rs:939-960`, `crates/sims-formats/src/tuning.rs` |
| `notifications.rs` | Text measured with cosmic-text (salvage finding 5, non-deterministic); text colours 0x4ff31a | **measurement replaced; colours still to mine** | Replaced by the deterministic `.ffn` bitmap fonts in `crates/sims-ui/src/font.rs` (no crate depends on cosmic-text). §3 item 8 |
| `dialog.rs`, `dialog/{neighbor,portrait/*,ui_bitmaps}.rs` | Job-popup atlas 4905 (22 rows; 0x530a7b); BMP_ language then default (0x419a5d/0x418e6c); portrait generators and colours; RT table lookup; 4-cell button crop; tutorial highlight | ported except tutorial highlight | `crates/sims-ui/src/balloon.rs:395,1105`, `hud.rs:113`; `crates/sims-people/src/caf_preview.rs:41-182`; `crates/sims-content/src/ui_graphics.rs:238`; `crates/sims-formats/src/ui_table.rs`. TutHigh: §3 item 10 |
| `native_bootstrap/` (26.8k; 47 of the 54 unique addresses) | PE loader, imports, fake kernel32/OLE32/GDI/USER32/WINMM, registry journal, mutex, FPU table, system collection | dropped | salvage-audit: sims-content DROP `native_bootstrap/`; vision (Win32 emulation). 0x5d91bc/0x666114 are InitializeCriticalSection; 0x6fe578 is the resource-manager critical section; 0x6017c8/0x5d5875 are collection allocations; 0x6c9f70 is mutex-name globals. Real facts in it are already ported: `lightmap.dat` size in `crates/sims-render/src/pools.rs:274`, ARRY ids in `crates/sims-sim/tests/integration/load.rs`, 24-bit FPU in `crates/sims-core/src/float.rs:271`. The `Scenario.tds` startup check has its source in legacy `sims-sim/native_world/resources/path.rs:8,811-821`, not here |
| `startup_memo.rs` (+dir), `startup_inputs.rs` (+dir), `owned_resources.rs` | Startup replay memo and tape; managed-save overlay | dropped | salvage-audit DROP list; REDB store replaces the overlay (`crates/sims-sim/src/neighborhood_store.rs`) |
| Remaining unique addresses outside `native_bootstrap` (0x405d5f, 0x418e6c, 0x419a5d, 0x52f2fc, 0x5a5752, 0x424c10/56, 0x4a0051, 0x4c4d7e, 0x4281df) | Portrait colour; BMP_ language; job atlas; family cleanup; surface catalog; example stack check | ported (behaviour) | Behaviour is cited in the rows above; only the address spelling is absent. 0x4281df is an example-only capture check (dropped) |
| `tests/` (83 files, 232 tests) | 65 files are fully gated (82 `#[ignore = private …]`, 9 `private-fixtures`; `tests/fixtures/` is absent, so not minable). 27 public tests in 12 files assert dropped infra. 30 public tests in 10 files encode knowledge | ported except §3 items 1, 9, 13 | Animation tables → `sims-vm/animator.rs:590-604`; OBJD 136/137 upgrade → `program.rs:1795-1823,2030`; cmx/bcf/ndx order → `import_wardrobe.rs:112-160`; loose over member and case folding → `crates/sims-content/tests/integration/install.rs`; CATS before CTSS → `buy.rs:1601`; GUID scope → `crates/sims-content/tests/integration/{catalog,neighborhood}.rs` |
| `examples/` (39) | 8 `compare_native_*` and 10 `export_*` are oracle tooling. 19 are private-content probes. `household_asset_report` and `expansion_asset_inventory` are superseded by `sims-cli census`/`install`. `find_guid` is a 32-line lookup with no rule | dropped | salvage-audit: sims-content DROP compare/export examples. `find_guid` could become an optional `sims-cli` subcommand (not knowledge) |
| In-`src` unit tests (652 `#[test]`, 25 files gated) | Mostly infra differentials (resource_scan, avatar_mount, retained, saved_types) | dropped / covered | See the rows above |

### 3. Still-to-mine backlog

Items 1-8 and 13 exist only in this crate (some also have a kernel in legacy `sims-sim`). Item 9's source is in legacy `sims-sim`; it is listed because a test here pins it.

1. ~~**Buy eligibility needs the actual TTAB (0x457cb0).**~~ **Ported:** `BuyDefinition::eligibility`, `interaction_table`, `master_of` and `has_interactions` in `crates/sims-content/src/buy_catalog.rs`, checked against 0x4cd83b–0x4cd86b, 0x45726a, 0x43ae0a and 0x457cb0. Switching `sims-ui` over is left to its track.
   - Rule: an object with no master and no graphic is listed only if its TTAB loads and has at least one entry.
   - The TTAB is looked up in the object's own file, then in its semi-global. A loaded empty table hides the fallback, and Global.iff is never read.
   - A TTAB id of -1 means the master's.
   - Current code checks only `ttab <= 0`.
   - Source: `legacy/crates/sims-content/src/object_catalog/products.rs:85-116`, kernel in `legacy/crates/sims-sim/src/object_catalog/products.rs:48-55`. Test: `tests/object_resources.rs::buy_product_table_fallback_and_legacy_names_use_actual_resource_precedence`.
   - Destination: `crates/sims-ui/src/buy.rs:1586` `is_buy_catalog_eligible`.
2. ~~**Catalog name fallback (0x4061e3, then 0x457d3b).**~~ **Ported:** `CatalogText::read`. The disassembly corrects two of the rules below: a `CATS` holds the description *then* the name (0x40631f), and a `CTSS` is read in the game's language only, with no per-entry fallback to language 1 (0x470ca7 with 0, 0). The `*` is at 0x6c12fc.
   - Rule: an empty CATS/CTSS name becomes `'*'` plus the OBJD label.
   - CTSS entries use the requested language, falling back to language 1, and are C-string-truncated by 0x5d4cf9.
   - Current code: `buy.rs:712-716` uses the label without `*`, and `crates/sims-vm/src/primitives/dialog.rs:1549-1552` says this fallback is not ported.
   - Source: `products.rs:34-76`.
   - Destination: `sims-ui` buy and `sims-vm` `catalog_name`. Verify the `*` against the exe.
3. ~~**Buy category labels.**~~ **Ported:** `BuyLabels` (0x5057a2–0x5058cb, 0x506765). Each table falls back to US English only when it has no string in the game's language (0x46feac with 0, 1). Catalogs 2–6 hide root buttons 4–6 (`BuyLabels::shown`).
   - Source tables: UIText STR#150 (7×8 roots), STR#200-207 (functions) and STR#210 (special columns 4-7 read entries 4, 5, 2, 3).
   - Catalog 6 rows 2 and 3 remap to `roots[5][4]` and `roots[4][2]`.
   - Source: `legacy/crates/sims-content/src/object_catalog/products/labels.rs`.
   - Destination: `crates/sims-ui/src/buy.rs`; there are currently no category texts.
4. **Built-in avatar materials (0x5d41db).**
   - Case-sensitive names checked before any texture: red, blue, black, grey/gray, yellow, green, default/white, BeigeMatte, Jeans, Shoes, T-shirt, Flesh, Roof.
   - Exact f32 bits for diffuse and ambient; specular and emissive 0; power 5; payload word 0 = 80.
   - Source: `legacy/crates/sims-content/src/avatar_material.rs:13-68`.
   - Destination: `sims-people` (`appearance.rs`/texture lookup) and `sims-render`.
5. **Footstep floor material (0x5890a8, 0x49fd95).** *Ported* (#485) to `crates/sims-vm/src/footsteps.rs` ([ADR](decisions/20261126-walk-animator-states-and-footsteps.md)). Open: plug-in floors, since sims-content keeps no category for floor patterns past `floors.iff`.
   - Water → 10.
   - Objects underfoot by GUID:
     - 0x96e3398d, 0xa2daa08c, 0xc02b406a, 0x65274a4f → 6;
     - 0xbf62f653, 0x7f907075 → 7;
     - 0x63416ba1 → 8;
     - 99999999 → 9;
     - 0x9be030b9, 0x3e7470f6, 0x5c67fc8f → 10.
   - On level 1, pool values 0-15 → 11 and 16 → 1.
   - Floor covering: 0 → 0 and 255 → 2. Other coverings use the first letter of the SPR2 label: C → 3, M → 1, S → 2, anything else 0. Coverings 1-29 come from `floors.iff`; the rest go through the house floor map.
   - Source: `legacy/crates/sims-content/src/character_loading/floors.rs:1-80`, kernel in `legacy/crates/sims-sim/src/pose/properties/surface.rs:13-58`.
   - Destination: `sims-vm` `clip_events`/`sounds` (unported per `sounds.rs:27-30`).
6. **Censor appearance.**
   - The appearance `<profile>-needs` is looked up inside `adult.cmx.bcf`, not in a file of that name.
   - The mask comes from person word 30, where bit 7 means 0x7f.
   - A binding is selected when the mask is all ones or `flags & mask != 0`.
   - Source: `legacy/crates/sims-content/src/character_initialization.rs:226-231`, `character_censor.rs`; kernels in legacy `sims-sim/pose/refresh.rs:96-115` and `pose/appearance.rs:9`.
   - Destination: `sims-people` (mesh) and `sims-render` (0x4ac707 mosaic; see `docs/decisions/20261118-goth-day-global-draws-are-censor-mosaic.md`).
7. **Object graphic selection; conflicts with current renderer.**
   - A selector at or above the graphic count wraps to 0 (0x438954). `crates/sims-render/src/scene.rs:3146-3151` skips drawing instead.
   - The DGRP view is the first whose direction **bitmask** contains the facing bit and whose zoom is exact (0x407847). `crates/sims-render/src/assets.rs:1209` uses direction equality.
   - SPR# wins over SPR2 for the same id (0x4c8e0d). `assets.rs:1218` reads SPR2 only, and `crates/sims-ui/src/thumbnail.rs:462` prefers SPR2.
   - The id interval comes from 0x45768a, with i16 wrap guarded.
   - Source: `legacy/crates/sims-content/src/object_graphics.rs:43-63,150-200`.
   - Destination: `sims-render` and `sims-ui`.
8. **Person notification text colours (0x4ff31a).**
   - default [0x000052, 0x00ca39], yellow [0xffcb17, 0x000000], magenta [0xff00ff, 0x000000].
   - Source: `legacy/crates/sims-content/src/notifications.rs:175-183`.
   - Two related rules live in legacy `sims-sim/person_notification/`, not this crate: removal at age 100 (0x437e9c) and locale currency `-$5` (0x6007df). The currency rule is ported (#484, `sims_ui::currency`).
   - Destination: `sims-ui`.
9. **Numbered catalog open order.**
   - Rule: for an enumerated `_17_Clothes.bcf`, the game first opens plain `Clothes.bcf`, and opens the numbered file only if that fails. Identity 0x80000000 means no prefix.
   - Current code reads the numbered file directly.
   - Source: legacy `sims-sim/pose/clothing/catalog_open.rs:44-80`, pinned by `legacy/crates/sims-content/tests/avatar_catalog.rs::startup_numbered_entry_tries_plain_name_before_its_enumerated_source`.
   - Destination: `crates/sims-content/src/import_wardrobe.rs:762-797`.
10. **Tutorial highlight (low).**
    - `CPanel/TutHigh.bmp` is 150×50 with three frames; the game uses the first 50×50.
    - Source: `legacy/crates/sims-content/src/dialog.rs:7-34`.
    - Destination: `sims-ui`.
11. **Skinning FPU control word (low, presentation).**
    - 0x037f vs 0x003f around skinning.
    - Source: `legacy/crates/sims-content/src/character_geometry.rs:252-262`.
    - Destination: a note in `sims-people`.
12. **Resize deletion condition (note).**
    - Rule: 0x442866 deletes an off-lot object only when both coarse X and Y are ≥ size-1, so negative coordinates are never deleted.
    - Source: `legacy/crates/sims-content/src/house_startup.rs:230-231`.
    - Destination: the `crates/sims-vm/src/load.rs:84-130` `resize` docs, when growing or occupied resizes are ported.
13. **Frame string context (test only).**
    - Rule: semi-global or global code reads STR# from the calling object's private file unless the operand asks for global.
    - Re-express `legacy/crates/sims-content/tests/frame_strings.rs::shared_code_loads_strings_in_each_private_frame_context` as a synthetic test in `crates/sims-vm/tests/`. The behaviour appears to be implemented (`interpreter.rs` `code_owner_type`) but no test covers it.

Optional, not knowledge:
- `examples/find_guid.rs` could become a `sims-cli` subcommand.
- The `compatibility.rs` operand and semi-global histograms could extend `sims-cli census`.

Once items 1-13 are ported, or recorded in `docs/`, the crate meets the "nothing left to mine" bar and can be deleted.

## sims-render — stays

Audited at `36fdb157` (main). Read-only; nothing was built or run.

### 1. Verdict: STAYS

Most of the crate's knowledge about the original game is already in the
current crates, and usually in a more faithful form. The current
`crates/sims-render` takes the original's integer projection, its depth
tables, cutaway, walls, floors, roads, pools and grass straight from the
original's code. The legacy versions were float approximations of the
same things. Skinning, the head overlay and portraits now live in
`sims-people`. Animation layering lives in `sims-vm`. Catalog and balloon
previews live in `sims-ui`, and picking lives in `sims-game`.
`camera_device/`, `dialog/`, the portrait jobs, `frozen_house` and the
examples are native-device shims or diagnostics. The vision drops those.

The one piece of real game behavior this audit found unported, the
**censor mosaic arithmetic** in `censor.rs` (0x4ac301, with the row quirk
at 0x4ac4c8/0x4ac649), is now ported (#486,
`crates/sims-render/src/censor.rs`).

Three smaller items are also open:
- the smooth-zoom and high-DPI composition design, which the vision
  requires and which is not built yet;
- edge rules in the object-preview fitting;
- a disagreement about the dialog object picture that needs checking.

So the crate stays. It can be deleted once the backlog below is ported,
or written into ADRs.

Address sweep:
- The crate's sources contain 122 distinct `0x4/5/6xxxxx` addresses,
  with or without the `0x` prefix. 38 of them appear nowhere in
  `crates/` or `docs/`.
- I sampled all 38. They are native device and widget plumbing
  (`camera_device`, `dialog/buttons`, object-preview surface binding
  0x4e5544/0x4e572c/0x5de95f, and test bookkeeping), except for these:
  - censor 0x4ac4c8 and 0x4ac649 (open, backlog item 1);
  - fit 0x4385bb, 0x4385c4 and 0x422665 (partly open, backlog item 3);
  - sprite blit 0x4ca78f (backlog item 5).
- The other addresses are aliases of ranges already ported (49c0e3,
  49cefc, 4aa54c, 438b4c, 4b4509).
- The the uncited-address list hit, 0x4bb08c, is a reflection-cycle
  assert in the private-fixture test `tests/portrait_resize.rs`. The
  native hangs on that input. Mirror indexing is ported
  (`crates/sims-people/src/caf_preview.rs:1052`).

### 2. Module table

| Legacy module/area | Knowledge | Status | Evidence |
| --- | --- | --- | --- |
| `lib.rs`: `Camera` (float iso projection `(x-z)*64, (x+z)*32 - y*78.38`, origin at 0.5/0.57 of the viewport, float depth `0.5-((x+z)*0.612+y*0.5)*0.02`) | Projection and depth | Superseded by the original's integer projection and depth tables | `crates/sims-render/src/camera.rs:1-16,140-160` (0x4b3c88, floor heights 58/116/232); `crates/sims-render/src/depth.rs:1-60`; ADR `20261114-render-uses-originals-depth-values.md` |
| `lib.rs`: `Camera::sprite_zoom` (thresholds 0.375/0.75 pick the authored zoom, then pixels and DGRP offsets scale together), resolution cap 4096 | Continuous zoom at any resolution (a client extension, not original behavior) | **Still to mine (design)**: vision.md:81 asks for "smooth zoom, high-DPI"; the current code has 3 discrete zooms plus integer pixel doubling only | `crates/sims-render/src/camera.rs:20-75`; `crates/sims-game/src/main.rs:1003-1014`; salvage-audit "Good news" bullet |
| `lib.rs`: DGRP layout (offset y = `s.y - height`, world offset `/16, /5, /16`, flag 1 flips, direction `rotate_left(rot*2)`, `(b&255)\|(b>>8)`) | Sprite placement | Ported | `crates/sims-render/src/scene.rs:3934-3935`, `assets.rs:1246`; ADR `20261114-hidden-objects-sprite-world-offsets-soft-edges.md` |
| `lib.rs`: `graphic_group` (0x45764f), `sprite_visible` (0x437f99), hidden flag data[34], container slot walk (0x437fdd), slot height `/(2√6)` (0x454f29) | Object draw selection | Ported | `scene.rs:4204-4215` (0x437f99), `scene.rs:3395-3420` (0x437fdd); `docs/decisions/20261121-paused-counter-wakeup.md:18` (0x45764f); `crates/sims-vm/src/primitives/reach.rs:197,523` (0x454f29) |
| `lib.rs`: DGRP sprite flag `& !5`, with the comment "bit 4 [value 4] is the luminous layer" | Lighting exemption (a claim; legacy never implemented it) | **Verify** (backlog item 5): the current code ignores flag 4 and lights every object sprite | `legacy/.../src/lib.rs:304-314`; `crates/sims-render/src/assets.rs:1246` |
| `lib.rs`: `actor_pose`, `visible_on_floor` (person word 40), R_FINGER0 carry, idle `tick*1.2`, placeholder floor/wall tints, obstacle boxes | Preview heuristics | Deliberately dropped (replaced by native animator, holder and level rules) | `crates/sims-vm/src/animator.rs:14-32,105`; `crates/sims-people/src/lot.rs:60-130`; legacy comment itself says "native draw filtering … not yet recovered" |
| `lib.rs`: `Scene::pick` | Pointer pick on sprite alpha and depth | Ported (native pick) | `crates/sims-game/src/game.rs:5409-5419` (`pick::at`) |
| `lib.rs`: `Assets::load_simulation` (placed definitions plus Create Object 0x2a GUIDs plus multitile plans) | Asset preload policy | Deliberately dropped (engine policy, not original behavior) | the current crate has its own asset loading in `crates/sims-render/src/assets.rs` |
| `camera.rs` (`from_native`, quarter-pixel fit to 0x49b0de) | Adapter from float camera to native camera | Superseded (the current camera is native) | `crates/sims-render/src/camera.rs` |
| `gpu.rs`, `world.wgsl`, `depth_range.rs`, `software.rs` (+`raster_context_tests`), `texture_tests.rs`, `decoded_sprites/` | wgpu composition with per-texel depth `z + (1-byte/255)*scale`, depth range refit, CPU reference raster, decode cache keyed by document bytes | Superseded: drawing moved to an exact CPU canvas (ADR `20261114-drawing-lot-starts-own-crate-cpu.md`). The GPU design is reference material for item 2 only | `crates/sims-render/src/canvas.rs`, `cache.rs`; salvage-audit "PORT gpu.rs/world.wgsl/software.rs" was overtaken by that ADR |
| `sprite_depth.rs` (0x4ca62e/0x4ca78f/0x49a5d3: `base + T[byte]`, `<=` wins, byte 255 skipped, flip, clip, rows with decoder AL≠1 skipped, depth scratch reset to 255 including clipped pixels) | Native sprite blit | Ported except the decoder row-skip detail | `crates/sims-render/src/depth.rs:20-35,379-430`; `canvas.rs:1312-1374` (`<=`); row-skip: backlog item 5 |
| `censor.rs` (0x4ac301/0x4ac707: blocks of 2/4/8 px by zoom, strict `z < depth`, per-block average + `(rand&15)-8` drawn R, G, B, 3 draws per block drawn, threshold row = clipped top + row-in-block (0x4ac4c8/0x4ac649), 555/565/556 `pack`/`unpack`) | Censor mosaic | ported (#486) | `crates/sims-render/src/censor.rs`; ADR `20261126-render-censor-held-order-walls-glide.md`. Legacy `Region::projected` padding (4 px / 12 px shift, seed `tick^id`) is **not** native: use the ADR's reading (person rect +0x230, pad 1/2/4, global stream) |
| `floors.rs` (global 23 lot size, drawn cells `1..size-1` 0x49cecc, split floor 255 to `styles[0]`/`patterns[0]` halves, background gray 112, edge-padding hack) | Floors | Ported (the padding hack and gray are GPU-only and dropped) | `crates/sims-vm/src/travel.rs:153,3505`; `crates/sims-render/src/scene.rs:1773,2239`; `lot.rs:45-50` |
| `floors/road.rs` (0x4a7dfe: 10↔11 swap on odd turns from the saved HOUS 0 phase, level 1 only) | Road dashes | Ported, test re-expressed | `scene.rs:4174-4192`, test `scene.rs:4997` |
| `floors/pool.rs` (0x4ad8ac 16 diamonds plus rim 12; 0x4614d6 nibble rotate; 0x461e39 corner table at 0x6e52a0; SPR2 1024+16z, 1072+4z) | Pools | Ported | `scene.rs:2001,3738-3760,5111`; `crates/sims-render/src/pools.rs`, `shadows.rs:116` |
| `grass.rs` (+`cache_tests`) (endpoints 0x49c0e3/0x49c8d3, 32 steps, x87 trunc 0x49cc39; GPU grain is a replacement) | Grass colors | Ported far beyond legacy (blades, jitter, shading) | `crates/sims-render/src/grass.rs:1-60` (same four colors) |
| `walls.rs` (`faces` pattern ownership, 0x53f6c1 diagonal sides, OBJD 0x5c style 22/23, 0xa2 shape base, object apertures per segment, corner-height order) | Walls | Ported (cited by name) | `scene.rs:1665-1680,2331-2356`; `assets.rs:71-73,788-789` |
| `wall_objects.rs` (0x4aa446..0x4aa54c temporary hiding, 0x4358c3/0x6f4670 normals), `wall_view.rs` (0x4a8895 flag passes, 0x400 flag) | Cutaway hiding of objects and walls | Ported | `crates/sims-render/src/cutaway.rs:651`; `scene.rs:525,3131,3940`; `grass.rs:617` |
| `pose.rs` (`skin`, `evaluate`, hierarchy, `head_overlay` 0x590b27, `carry_overlay` prio 10, `background_underlay` prio -10, locomotion retarget heuristics) | Skeletal pose | Ported (`skin`, head, layers); locomotion retarget heuristic deliberately dropped | `crates/sims-people/src/skin.rs:1-10`, `rig.rs`; `crates/sims-vm/src/animator.rs:14-32,105,238`; `crates/sims-vm/src/head.rs:368-410` |
| `object_preview/fit.rs` (0x4384c8: zoom 3→1, margin ≥ 3, signed IDIV facing `(dir+4)%8` 0x4385bb/c4, centering `(b0-b2+target)/2` with wrapping, explicit zoom verbatim, multipart non-lead skip) | Isometric object preview fitting | Mostly ported; **edge rules still to mine** | `crates/sims-ui/src/thumbnail.rs:145-175,366-375` (uses `rem_euclid`, image size not the measured rect union, rejects zoom outside 1..3) |
| `object_preview/software.rs` (0x438ae4..0x438b58 placement and `__ftol` rise, rect union 0x422665, 120×120 cell in a 125×127 surface) | Preview sprite placement | Placement ported; 120×120 dialog use **to verify** | `scene.rs:3494-3505`; `thumbnail.rs:375`; see item 4 |
| `object_preview/surface.rs` (0x4e5544/0x4e572c target binding) | Native draw-target globals | Deliberately dropped (native memory and device mirroring; vision) | `docs/vision.md` dropped list |
| `object_speech.rs` (0x4cc3b9), `portrait/` (cache, job, retrieval 0x4e92a8, persistence 0x4e9000, speech 0x4e915d, generate; `blit/bmp/images/resize` are re-exports) | Image-cache services, portrait regeneration jobs | Deliberately dropped (salvage-audit: PARK/DROP "portrait jobs"). The knowledge itself is ported: generators, crops, 5-6-5, Hermite resize, mirror | `crates/sims-people/src/caf_preview.rs:1-40,950-975,1052`; `crates/sims-sim/src/world.rs:454-480,651`; `crates/sims-render/src/neighborhood.rs:2225` (0x4ba79c) |
| `dialog/` (`dialog.rs`, `client.rs`, `client/buttons.rs`, `client/live_panel.rs`) | Suspended dialog-constructor owner, resident buttons, Needs panel; portrait layout `[45,45,5,2,35,41]` with templates 108/4600; non-person dialog resource drawn as a 120×120 isometric preview | Deliberately dropped (salvage-audit PARK/DROP `dialog/`). MOTIVES order, 51b7f6, 518665 and 47689c are ported elsewhere. The **120×120 object picture is a disagreement to check** (item 4). The atlas layout belongs to legacy sims-content (0x4e7082) | `crates/sims-ui/src/dialog.rs:1260-1300,519-580`; `crates/sims-ui/src/hud.rs` (51ce79/51c5a6); `crates/sims-people/src/caf_preview.rs:968-970` (45×45 with 35×41 at (5,2)) |
| `camera_device/` (`camera_device.rs`, `bind`, `mode` 0x4d31b7, `normal_frame` (+tests), `properties`, `viewport` 0x4b136b, `panels/` 0x50a8ec→0x53ab6d→0x53b536, `catalog` 0x507288/0x506765) | Native widget and device owners driving legacy sims-vm/runtime state machines | Deliberately dropped (salvage-audit PARK/DROP; vision drops native call-graph mirroring). "Mine panels for layout": they hold **no pixel layout** ("not the legacy window's pixel width"). Catalog filtering and availability are ported | `crates/sims-ui/src/buy.rs` (507288, 506765); `crates/sims-ui/src/hud.rs` (53b536); `docs/decisions/20261121-buy-mode-catalog-placement-and-chair-replay.md` |
| `frozen_house.rs`, `original_house.rs` (+`ground_reuse_tests`) | Read-only presenter over the legacy Session; indoor-room camera framing; terrain `h/(2√6)` | Deliberately dropped (salvage-audit PARK/DROP `frozen_house`); terrain superseded (16 near px per step) | `crates/sims-render/src/camera.rs:14-15`; `scene.rs:1665-1680` |
| `tests/` (94 files) | 86 are `#[ignore]` (private game data and checkpoints under `cache/`) or private-fixture-gated (`native_censor_pixels`, `native_sprite_depth`, `native_graphic_lookup`, `native_sprite_visibility`, `portrait_bmp*`, `portrait_generators`, `portrait_resize`). Fixture JSON is absent, so they cannot be mined | Not minable, except as recipes | `grep -c '#\[ignore'`: 86 of 94 files; no `tests/fixtures/` dir |
| `tests/` live synthetic | `object_preview.rs` (7 live fit tests encoding 0x4384c8 rules), `render.rs` (skinning blends after transform; head overlay subtree only), `native_camera_projection.rs` (float vs native ≤ 0.25 px), `screenshot_compare.rs` (11 tests of diff tooling), live parts of `household_player_continuation`/`player_action_sequences`/`action_thumbnail`/`dialog_client` | Fit tests: **re-express** (item 3). Skin test: optional re-expression in `sims-people/src/skin.rs` (no unit test there today). The rest test legacy approximations or tooling and are dropped (vision: evidence/receipt tooling) | `crates/sims-people/src/rig.rs:172-290` covers the head and hierarchy |
| `examples/` (16 plus `support/`: `head_look_frames` 2k, `newbie_parity`, `family_window`, `screenshot_compare`, …) | Headless diagnostics and a window client | Deliberately dropped (salvage-audit PARK/DROP "the examples") | current oracle and ratchet tooling: `crates/sims-game/tests/ratchet.rs`, `ui_oracle_tour.rs` |

### 3. Still-to-mine backlog

1. **Censor mosaic.** *Ported* (#486, [ADR](decisions/20261126-render-censor-held-order-walls-glide.md)).
   - Source: `legacy/crates/sims-render/src/censor.rs`, functions `apply`,
     `pack` and `unpack`. Port these.
   - Destination: `crates/sims-render` as a post-pass over the
     lot-plus-people framebuffer and its depth. It runs from
     `sims-people::draw_on_lot` and `sims-game` for each person whose
     data word 30 is set.
   - Rules to keep:
     - block size 2/4/8 by zoom;
     - a pixel is drawn only when `z < depth` (strict);
     - the threshold row is the clipped top plus the row within the
       block, even for later blocks (0x4ac4c8/0x4ac649);
     - noise is `(rand & 15) - 8`, drawn for R, G, B in that order,
       three draws per block that is drawn, taken from the **global**
       stream after the lot;
     - divide with truncation, then clamp.
   - Do **not** port the legacy `Region::projected` padding or its seed.
     Use the reading in `docs/decisions/20261118-goth-day-global-draws-are-censor-mosaic.md`
     (rect +0x230, pad 1/2/4).
   - Also record the 5-5-5, 5-6-5 and 5-5-6 pack/unpack formulas.
2. **Smooth zoom and high DPI (a design reference, not original behavior).**
   - Sources:
     - `legacy/crates/sims-render/src/lib.rs:418-431` (`sprite_zoom`
       thresholds 0.375/0.75; pixels and DGRP offsets scale together);
     - `src/gpu.rs` and `src/world.wgsl` (per-texel sprite depth);
     - `src/depth_range.rs` (one affine depth refit that keeps ties).
   - Destination: `crates/sims-render` and `sims-game`, for the
     "smooth zoom, high-DPI" line in `docs/vision.md:81`, which is not
     built (`sims-game/src/main.rs:1003-1014` has integer doubling only).
   - Alternatively, write the approach into an ADR and drop the code.
3. **Object-preview fitting edge rules.**
   - Source: `legacy/crates/sims-render/src/object_preview/fit.rs` and
     the live tests in `tests/object_preview.rs:20-105`.
   - Destination: `crates/sims-ui/src/thumbnail.rs` and its tests.
   - Rules to check and port:
     - facing is signed IDIV `(dir+4)%8`, so -7 gives -3; the current
       code uses `rem_euclid`;
     - margins and draw offsets come from the measured union of sprite
       rects (0x422665), with wrapping subtraction and division that
       truncates toward zero;
     - a zoom the caller supplies is used as given;
     - non-lead multipart pieces return zoom 1 with no bounds;
     - the output rect is the draw result, not the measured rect.
4. **Dialog object picture (verify, then port).**
   - Legacy: `legacy/crates/sims-render/src/dialog.rs:70-90,159-187`
     draws a dialog's non-person portrait resource (definition kind ≠ 2)
     as an isometric preview. It is fitted by 0x4384c8 into a 120×120
     cell on a 125×127 keyed surface.
   - Current: `crates/sims-ui/src/dialog.rs:519-580` (icon mode 0) uses
     the 45×45 catalog icon or thumbnail instead.
   - Settle which is right against an original screenshot of an object
     dialog. The legacy test `tests/object_preview_dialog.rs` (Newbie
     shower) is the recipe.
   - Destination: `sims-ui`.
5. **Small checks.**
   - (a) `src/sprite_depth.rs:60-112`. The native blit skips a color row
     when the decoder's per-row result (AL) is not 1. It also resets the
     depth scratch to 255 for every row it processes, clipped pixels
     included. Confirm that the current SPR2 decode and blit
     (`crates/sims-render/src/canvas.rs:1151+`) cannot differ.
   - (b) `src/lib.rs:304-306` claims DGRP sprite flag 0x4 marks a
     "luminous" layer. The current renderer ignores the flag and lights
     every object sprite (`light.rs`). Check with a lamp or TV at night.
   - (c) Optional: re-express `tests/render.rs:218-267` ("skinning blends
     after transform") as a unit test in `crates/sims-people/src/skin.rs`.

#### Housekeeping for a later deletion
- `.gitignore:65,67` and `scripts/check_repository.py:30,32` name
  `legacy/crates/sims-render/tests/{clown_household_continuation,fresh_household_replay}.rs`.
- About 10 doc comments and ADRs cite `legacy/crates/sims-render/...`
  paths. When the crate is deleted they are rewritten to `legacy-final:crates/sims-render/...`.

## sims-vm — stays

Audited at `36fdb157` (origin/main). Read-only: grep and reading only, nothing built.

### 1. Verdict: STAYS

The interpreter core and its pure kernels have been mined thoroughly. That covers rng, numeric, memory/expression/Random/scopes, motives, autonomy (CRT qsort and top-N), `call()` argument semantics, the action queue, find-best, relationship, the person factory and reset, the loaders, multitile, removal, headline, look, reach, dialog text, and Generic Sims Call. The current `crates/sims-vm` (87k lines) cites the legacy crate in ~64 places and covers more than it did. Every primitive opcode 0–37 and 41–51 now has an arm in `crates/sims-vm/src/primitives.rs:129-177`, including the ones the salvage audit listed as missing: 15, 19, 21 and 33. Generic Sims Call implements 2, 6, 10, 11, 12, 14, 15, 18, 26, 28, 35 and 43. That is a superset of legacy, which had only 2, 6, 10, 11, 12, 14, 15, 28 and 43, so legacy has no further Generic subfunctions to mine.

Some knowledge was still unported when this audit was written: a few pieces of real game behavior and some exact ordering rules. Examples were community-lot RNG reseeding, the date indicator with its notification queues, dialog named icons (icon mode 4), and the MSVC `std::sort` tie order used for draw order. Most of these are now ported (backlog below), including the sort's use for the wall draw order (#486); the terrain gates remain, and two more areas are parked. The crate therefore **STAYS**.

Mechanical check. Of the 231 addresses in the uncited-address list, ~175 sit in the DROP-listed native drawing and interface machinery: `scheduler/clock/draw/targets/**` (blit/unlock), `draw/background`, `draw/context/placement` stage cuts, `interface/view_control` query call sites, and profiler timestamps. The remaining ~55 resolve to the backlog items below, or to behavior already ported under other addresses. I also extended the check to bare-hex addresses (`445449` without `0x`), per file, in `scratchpad/vm-file-addr-coverage.txt`. Sim modules are 80–100% cited. The uncited residue is concentrated in `presenter.rs`, `nearby_houses.rs`, `indicator_refresh.rs`, `selection_indicators.rs`, `dialog.rs` (0x4368b6) and `career_text.rs` (0x403d64).

### 2. Module table

| Legacy module/area | Knowledge | Status | Evidence |
| --- | --- | --- | --- |
| `lib.rs` (interpreter, dispatch, call) | step loop, branch, `call()` contiguous args, `[255;8]` temps | ported | `crates/sims-vm/src/interpreter.rs:7` (cites lib.rs:1550-1800), `:1537-1548`; `primitives.rs:58` |
| `rng.rs` | 0x41c64e6d lot/global stream, CRT `rand` 0x343fd | ported | `crates/sims-core/src/rng.rs`; `interpreter.rs:40` |
| `rng.rs` `load_saved` | 0x46d09c..0x46d0e8: a nonresidential lot reseeds both streams from two timer readings | **ported** | `sims_vm::boot::read_simulator` reseeds a nonresidential lot's streams (`World::seed_from_host_clock`, from the house's saved tick count); used by `Lot::load`, travel's destination load and `lifecycle.rs`'s base-house load. Before the port, `World::read_simulator` always restored the SIMI words |
| `numeric.rs` (`Wide`, ftol) | 64-bit-significand intermediates, half-even | ported (rewritten) | `crates/sims-core/src/float.rs` (`Ext`, `F32`) |
| `memory.rs`, `resource_storage.rs` | scopes, cells, Expression, Random, TATT resource globals | ported | `crates/sims-vm/src/memory.rs:8` (cites memory.rs:137-746), `primitives/expression.rs:3`, `primitives/random.rs:3`, `memory.rs:566` |
| `motives.rs`, `motives/score_owner.rs` | curves (0x41683c ordering), decay, the 0x45cdac update block, score | ported | `crates/sims-sim/src/motives.rs:15,165,241,379`; `primitives/motive_change.rs:27` |
| `autonomy.rs`, `autonomy/query.rs`, `autonomy_constants*` | CRT qsort ties, top-N, seated reject, join scan (0x41ee99), FCNS constants | ported | `crates/sims-vm/src/autonomy.rs:41-56,339-385,800`; `program.rs:944-971` |
| `action.rs`, `functional.rs`, `named.rs`, `immediate.rs` | eight-slot queue, truncation at 7 (0x45d3c1), push, found action, named and functional trees | ported (`immediate` dropped as a host boundary) | `primitives/in_way.rs:481-635`, `push.rs:44`, `found_action.rs:35`, `named_tree.rs:5`, `run_functional_tree.rs`, `object_uses.rs:24` |
| `person.rs` | Generic Sims Call, action string, inventory, Set To Next, tile depth | ported | `primitives/generic.rs:5`, `action_string.rs:66`, `inventory.rs:76`, `set_to_next.rs:4,229,448` |
| `find_best.rs`, `placement.rs`, `routing.rs`, `reach.rs`, `look.rs`, `transfer.rs` | 0x44f2bf, Find Location, 0x45c634/0x4520e9 relative routes, 0x45c9ba, 0x58f4ee turns, grab/drop | ported | `primitives/find_best_object.rs`, `placement/find.rs`, `routing/slot.rs`, `sims-sim/src/route.rs`, `primitives/reach.rs`, `look_towards.rs:28`, `grab.rs:37` |
| `relationship.rs`, `family*.rs`, `person_friends.rs`, `neighborhood.rs`, `magic_history.rs`, `fires.rs`, `zoning.rs`, `world_map.rs` | 0x44d17c rows (insertion order), leave family 0x42601e, friend counts 0x427148/0x427219, family reload 0x424b9f, magic history 0x410af6, fire list, zoning, lot resize | ported | `primitives/relationship.rs:5`; `sims-sim/src/neighbors.rs:70,300-330,556,602`; `generic.rs:10-43`; `removal.rs:538`; `sims-content/src/neighborhood.rs:175`; `crates/sims-vm/tests/integration/resize.rs` |
| `construction.rs`, `create.rs`, `multitile.rs`, `group_factory*`, `cursor*`, `lifecycle.rs`, `global_load.rs` | object factory, countdown word 0x431dc9, create modes, MT offsets, cursor 0x421afb, Init/Main, 0x442b9f | ported | `factory.rs:20,380,488,548-573`; `objects.rs:24`; `primitives/create.rs:5`; `group_factory.rs:41`; `placement.rs:31`; `lifecycle.rs`; `load.rs:356` |
| `person_factory*`, `person_reset*`, `person_loading.rs`, `loading.rs`, `load_completion.rs`, `load_finalize.rs`, `body_profiles*`, `career_appearance.rs` | dynamic person construction, 0x45e791 reset order, post-load (0x459bf6), score tables, use-list record, 0x428041/0x43fc94, BodyStrings 0x457f60, 0x45ee5f cache | ported | `person_factory.rs:33`; `people.rs:6-10`; `objects.rs:29,119`; `visits.rs:38`; `object_uses.rs`; `sims-sim/src/load.rs:547,900` |
| `removal.rs`, `deletion_entry.rs`, `indicator_unregister.rs`, `indicator_pools.rs`, `load_indicators.rs` | 0x451b6a/0x442d70, 0x440eeb, 0x447d8b→0x448097, 0x4481eb, star pool 0x446356, 0x445e4a | ported | `crates/sims-vm/src/removal.rs:21,213,511`; `placement.rs:249,2028`; ADR `20261122-play-mode-end-of-update-...` |
| `selection.rs`, `selection_indicators.rs`, `placed_indicators.rs`, `indicator_refresh.rs` | selection 0x445449, placed 0x44752e, star marker | partly ported | `crates/sims-vm/src/selection.rs:13,303`; `placement.rs:1790-1895` |
| same (date path, queues) | date indicator (Entry::Date, 0x4472a2/0x446ee9, 0x4477fb); word 77/81 write-intent queues 0x4448a6/0x444962; drain 0x4464db/0x448847 | **ported** | `selection.rs` `refresh_indicators`, `refresh_date`, `refresh_star`; `placement.rs` `mark_star`; `sims-sim/src/world.rs` `IndicatorQueues`. Before the port, the date path failed loudly and travel masked fame and date around selection |
| `idle.rs`, `burn.rs`, `budget.rs`, `effect.rs`, `headline.rs`, `animation.rs`, `ui_owner*`, `ui_effect.rs`, `refresh.rs`, `registry_flags.rs`, `player_menu.rs`, `program.rs`, `compatibility.rs`, `slot_geometry.rs` | Sleep/Notify, Burn, budget 0x46d91c, special effect 0x44aaa6, headline 0x416a6f, animation events 0x58950b, events, UI Effect, refresh plus room signals F1/F2/F3, 0x4358fc, pie menu 0x4582ad, linker, opcode names | ported | `primitives/{sleep,notify,idle,burn,budget,special_effect,headline,animate,events,ui_effect,refresh}.rs`; `sims-sim/src/rooms.rs:866-945`; `orders.rs:9-24`; `program.rs`; `primitives.rs` NAMES |
| `dialog.rs`, `dialog/` | 0x451552 protocol, text passes, buttons, resource icon modes 0/2/3, time and neighbor dialogs | ported | `primitives/dialog.rs:541`; `sims-ui/src/dialog.rs:41,268,1261-1321`; ADR `20261114-message-dialogs-...` |
| `dialog.rs` `named_icon`, `job_icon_rect`, `GlobalBitmap` | icon mode 4: `job N` → 22-row atlas (0x530a7b, -1→row 11); `gz` = first of four button cells, `gzi` = whole image (0x4368b6); CRT-atoi parsing (0x46f874/0x46f891) | **ported** (also `my`, `guid`, `rel`, which legacy left out) | was: `sims-ui/src/dialog.rs:1298-1321` mode 4 only formats text and sets `icon = None`; the atlas exists in `sims-ui/src/balloon.rs:191`, `hud.rs:112` |
| `career_text.rs` | 0x403d64: titles, ordinal 10 skipped | partly ported | `sims-ui/src/household.rs:807-880` |
| `career_text.rs` (rest) | STR# `id+1000` alternate title table; per-level secondary strings (entries `4+2i`, `24+i`); offers 2/3 | **ported** (both title pairs: 0x4033d2 for the job panel and `$Job`, 0x4033a4 for the 0x52ff7e list box); secondary strings have no known reader; the 11/16/21 expansion gate stays documented | `sims-ui/src/household.rs` `Careers::load_with_pool` |
| `nearby_houses.rs` | MSVC `std::sort` (introsort, guarded/unguarded insertion; tie choices 0x4306b0, 0x430a95, 0x430f0a, 0x430fb7, 0x54186d); nearby-lot order 0x428350/0x42ffce | **ported** (the sort: `sims-core/src/sort.rs`, checked against the 0x449cbc instantiation); nearby-lot order deliberately empty | `set_to_next.rs:58-90` documents an empty vector in every capture; the sort's port is `sims-core/src/sort.rs` |
| `presenter.rs`, `presenter/redraw.rs` | draw-state propagation 0x434657, 0x4226b6, 0x4aacb9 | ported | `sims-render/src/scene.rs:118,553-614` |
| `presenter.rs` (order) | object draw-order cache (0x449cbc): rotated key `(x+y, x−y)`, `outside` 0x433087, MSVC tie order; wall-record sort 0x540809/0x541346/0x541389 | **still to mine** (render track; the sort is in `sims_core::sort`) | render approximates: `scene.rs:2376-2400` (`sort_by_key`), `scene.rs:3117` (depth, id) |
| `snapshot.rs`, `native_registry_reentry.rs`, `native_suspension.rs`, `registry_suspension*`, `retained_completed*`, `retained_epoch.rs`, `retained_frames*`, `action_observation*` | resumable native-continuation import and observation | deliberately dropped | salvage-audit (VM DROP list); vision "Non-goals" (no matching of internal layout or call order); snapshot format: salvage finding 8 and the new `sims-sim/src/snapshot.rs` |
| `scheduler.rs`, `scheduler/{storage,registry,visit,transport}.rs` | 0x46d553 registry pass, countdown words, holes | ported in substance (`tick.rs`); storage model **parked** | `crates/sims-vm/src/tick.rs:6,64`; salvage-audit "PARK scheduler storage model" |
| `scheduler/clock.rs`, `clock/{fps,outer,post_clock,profiler*,application}.rs` | wall-clock timer 0x4732c0, FPS, profiler | deliberately dropped | salvage-audit "DROP scheduler/clock"; vision "no wall-clock reads" |
| `scheduler/clock/speed_override.rs` | 0x46d790/0x46d7db | ported | cited in current crates (0x46d790 ×3, 0x46d7db ×4) |
| `scheduler/clock/draw/{ground,context/light,context/placement,actions/**}` | ground draw, room light, sprite placement 0x4b9613→0x4b97ea→0x4c9ceb, action-queue icons 0x4d28df | ported | `sims-render/src/depth.rs:19,195,284`, `light.rs:7`, `scene.rs:2121,2826,3079`, `grass.rs`; ADR `20261115-selected-sims-actions-...` (`sims_ui::queue`) |
| `scheduler/clock/draw/ground/visible.rs` (gates) | 0x49cf02..0x49d237: forced-shade mode (+0x37c), rim tiles shade 0 (+0x8d), snow gate (+0x81, terrain ≥ −97), wall-flags `&0x30` gate | **still to mine** (verify; low) | `sims-render/src/grass.rs` has no rim, snow or forced-shade rule; terrain presets are noted as unported at `grass.rs:171`. The grass's constructor clears +0x81 and +0x8d (0x49c1e0, 0x49c26a). Only the preset setters (0x49e27b, 0x49e379) and a per-tile flag (0x49ce5e → +0x37c) set them, so this goes with the presets |
| `scheduler/clock/draw/{targets/**,background,regions/**,framework,view,actions/image/**}` | DirectDraw surfaces, dirty rectangles, blits | deliberately dropped | vision "Non-goals" (Windows APIs, internal layout) |
| `interface.rs`, `interface/**` (7.4k) | native CPState/ViewControl/Buy/Options publication callers | deliberately dropped | salvage-audit DROP `interface`; mode change 0x4cea5d, live panel 0x51d375/0x51ce79, pause 0x4cee4a and others are ported in `sims-ui`/`sims-game` (cited 2–15×) |
| `interface/live_panel/history.rs` | `cAverageHistory` 0x4cb4a5/0x4cb5d6/0x4cb58a/0x4cb635 (ties away from zero, bounded trend) | **ported** (`sims-ui/src/needs_history.rs`). The consumer is the Needs bars: one history per member (10, 5) at 0x513aeb, fed from the need floats at 0x513b41, trend to the bar's +0xde at 0x50c66f. Not drawn yet | |
| `ui_effect/widgets/**` (3.3k) | cTSWin window manager, timers, transitions | deliberately dropped | salvage-audit DROP `ui_effect`; vision (UI is new, winit/wgpu) |
| `image_cache.rs`, `image_cache/**` | portrait cache 0x4eb0eb/0x4e895e/0x4e8c5d/0x4e92a8, speech images | ported / dropped | portraits: `sims-content/src/family_import.rs`, `sims-sim/src/world.rs`; balloons: `sims-ui/src/balloon.rs:181,927`; persistence to the original file is a vision non-goal (not writing the original save format) |
| `commander.rs`, `commander/house*` | 0x407577/0x4075df/0x4075a1 registration; 0x40760d broadcast (0x40760d is cited 8×); house-listener branches F6/E4/E2/F9/84/C2/DA by receiver | **parked** | salvage-audit "PARK commander" |
| `tests/` (171 files, 38.5k lines) | 127 files `include_str!` from `tests/fixtures/*.json` (161 files), none present; the rest are synthetic | mostly not minable; 2 tests worth re-expressing | `dialog_named_icons.rs` (backlog 3) and `live_panel_history.rs` (backlog 6). `relationship_order.rs`, `native_neighbor_friends.rs`, `native_global_load.rs` and `cursor_construction.rs` encode behavior already ported. `scheduler*.rs`, `interface_*`, `window_*`, `ui_timer_dispatch.rs` and `action_execution_observation.rs` assert native bookkeeping (dropped) |
| in-src tests (`*_tests.rs`, `*/tests.rs`) | the same split | not minable (fixture-gated) or dropped | `person_reset_tests` and `memory_*_tests` include fixtures; `retained_*`/`action_observation_tests` cover dropped areas; 4 are gitignored (`.gitignore:19-25`) |
| `examples/compare_native_*.rs` (8) | native capture comparators | deliberately dropped | vision Verification (QEMU oracle replaces these); salvage-audit drops compare/export examples |

### 3. Still-to-mine backlog

1. **Community-lot RNG reseed on load.** *Ported.* Source: `legacy/crates/sims-vm/src/rng.rs:65-81` (`load_saved`, 0x46d09c..0x46d0e8). A nonresidential lot replaces both the lot and global streams with two consecutive timer readings. A residential lot keeps the SIMI words.
   - Destination: `sims-sim` `World::read_simulator` and its callers in `sims-vm` (`travel.rs`, `boot.rs`, `lifecycle.rs`).
   - Make the readings seeded inputs recorded in the save (salvage-audit decision 3). See also ADR `20261118-family-import.md:52`.
2. **Date indicator and indicator notification queues.** *Ported.* Sources:
   - `legacy/crates/sims-vm/src/indicator_refresh.rs`: write-intent queues for person words 77 and 81 (0x4448a6/0x444962), the ordered drain 0x4464db and the 0x448847 clear.
   - `selection_indicators.rs`: `Entry::Date`, `DateDetach`/`DateTarget` (0x4472a2, 0x446ee9).
   - `placed_indicators.rs`: the 0x4477fb branch.
   - `indicator_unregister.rs` `Queues` (+b0/+bc).

   Destination: `sims-vm` `selection.rs` and `placement.rs`, and the `memory.rs` write path. Now ported there; `travel::select_sim` no longer works around them.
3. **Dialog icon mode 4 (named icons).** *Ported.* Source: `legacy/crates/sims-vm/src/dialog.rs:24-35,405-470`:
   - `named_icon`, with case-insensitive category parsing and CRT atoi;
   - `job_icon_rect` (0x530a7b, school −1 → row 11);
   - `GlobalBitmap` `gz`/`gzi` (0x4368b6).

   Test: `legacy/crates/sims-vm/tests/dialog_named_icons.rs`. Destination: `sims-ui/src/dialog.rs`, mode-4 branch.
4. **MSVC `std::sort` with exact tie order, and the original draw order.** *The sort is ported* to `sims_core::sort`, and the wall draw order uses it (#486). The object `Order` cache's tile key (legacy `rebuild_order`) was tried and **not adopted**: it drew 6 ratchet pixels unlike the original, which the depth order matched (see the #486 ADR's table). Sources:
   - `legacy/crates/sims-vm/src/nearby_houses.rs:20-120`: introsort, partition and heap tie choices, insertion pass.
   - `presenter.rs:15-176`: `sort_wall_records` (0x540809/0x541346/0x541389, comparator 0x44054c), the object `Order` cache (0x449cbc), `draw_key` and `outside` (0x433087).

   Destinations:
   - the sort as a utility in `sims-core`;
   - `sims-render/src/scene.rs`, replacing the `sort_by_key` approximations at `:2400` (walls) and `:3117` (objects);
   - `set_to_next.rs` mode 11, if a neighborhood ever fills the lot-position table.
5. **Career text extras.** *Ported.* Source: `legacy/crates/sims-vm/src/career_text.rs:38-93`: the alternate STR# `id+1000` title table, per-level secondary strings, and the `11/16/21` count by expansion level. Destination: `sims-ui/src/household.rs` `Careers`. Minor.
6. **`cAverageHistory`.** *Ported*, with its consumer identified (see the table). Source: `legacy/crates/sims-vm/src/interface/live_panel/history.rs`: ring of samples with stored averages, average rounded half away from zero (0x4cb58a), trend bounded by ±limit (0x4cb635); the `Elapsed` interval consumer (0x473311). Test: `tests/live_panel_history.rs`. Destination: `sims-ui`, once the owning panel widget is identified. Minor.
7. **Terrain-preparation gates.** Source: `legacy/crates/sims-vm/src/scheduler/clock/draw/ground/visible.rs:200-330`:
   - forced shade (+0x37c);
   - shade 0 within 1 tile of the active lot edge (+0x8d);
   - snow child when +0x81 is set and terrain ≥ −97;
   - wall flags `&0x30`.

   Destination: `sims-render/src/grass.rs`. Verify against the oracle when terrain presets (`grass.rs:171`) are ported. Low.
8. **Parked, keep for reference:**
   - `legacy/crates/sims-vm/src/commander.rs` and `commander/house*`: listener registration and broadcast order, per-receiver message branches.
   - `scheduler.rs` and `scheduler/{storage,registry,visit,transport}.rs`: the registry storage model.

   Destination: `sims-vm`/`sims-game`, only if the oracle shows that broadcast or registry order is observable.

Not legacy-minable: the Generic Sims Call subfunctions that are still missing. Legacy implemented none beyond the current set, so they need fresh work from the executable.

Housekeeping for whenever this crate is deleted: `.github/repository-policy.json:44`, `.gitignore:19-25` and `scripts/check_repository.py:34-40` reference paths inside it.

## sims-sim — stays

Audited at 36fdb157 (main). This was a read-only audit: grep and sed only, with no builds.

### 1. Verdict: **STAYS**

Most of the crate has been mined. That includes the areas `docs/salvage-audit.md` asked to refactor and keep:

- routing: regions, portals, the room-to-rectangles step, the A* planner, contention (shoo) and candidate slots;
- placement and Find Location;
- the clock and the day's turn;
- house statistics;
- lot_environment;
- the saved_* importer;
- the pose kernels.

These now live in `crates/sims-vm` (routing/, placement/, primitives/, animator.rs, tick.rs, daily.rs, removal.rs, buy.rs, build.rs), `crates/sims-sim` (rooms.rs, load.rs, head.rs, pose.rs, animator.rs, world.rs), `crates/sims-render`, `crates/sims-people` and `crates/sims-content`. 48 current files cite legacy sims-sim paths.

Across the 135 legacy modules I matched every 6-hex native address, written with or without `0x`, against `crates/` and `docs/`. All the rule-bearing modules have 0–3 addresses not found. The large unmatched clusters are in native_application (132), native_game (136), native_system (69), native_world (56), native_wall_io (50) and lot_presenter (115). They are resume points, vtables, memory-write sites and Win32 plumbing, which the vision drops.

About 19 concrete game rules are still unported, listed in §3. Among them:

- the walk animator's states 5, 7, 8 and 12 (ported since, #485);
- footstep and clip-sound selection (ported since, #485);
- route-cache invalidation on house refresh;
- the "Go Here" ground click;
- the architecture value;
- the original wall draw order (ported since, #486);
- the linear camera glide (ported since, #486).

The crate therefore stays. After the §3 items are ported it should be DELETE-SAFE: everything else is ported, or dropped under `docs/vision.md:22-25` (no Win32 emulation, no matching of internal layout or call order) and `docs/salvage-audit.md:187-193`.

**Tests.** 176 of the 206 files `include_str!`/read `fixtures/*.json` or `SIMS_*` oracle reports. All 266 referenced fixture paths are absent from the checkout, so those tests cannot be mined.

About 30 files are synthetic. The ones that encode real rules already have equivalents:

- room diagonals and partition: `crates/sims-sim/tests/integration/rooms.rs`;
- water access and diagonal seeds: `crates/sims-vm/src/routing/rooms.rs`;
- a paused pass: `crates/sims-vm/src/tick.rs:686`.

The rest test continuation or edit-history bookkeeping.

### 2. Module table

Status key: **ported**, **dropped** (vision or salvage-audit), **to-mine** (see §3; the number is the backlog item).

| Legacy module/area | Knowledge | Status | Evidence |
|---|---|---|---|
| `routing.rs`, `routing/{standing,posture}.rs` | 0x483414 standing and seated candidate scan, random subset limit (seats kept first), 0x4832d4 departure, 0x45c38e endpoint look, needs-portal | ported | `crates/sims-vm/src/routing/slot.rs:1-24,777,976-979,1356-1381` |
| `regions.rs` | 0x469b26 free-rectangle growth; first hard / last soft overlap | ported. The aspect limit is deliberately left out because the planner never sets it | `crates/sims-vm/src/routing/regions.rs:1-7` |
| `region_path.rs`, `path_world.rs` | A* over goal-labelled rectangles, smoothing, free cost, step limit | ported | `crates/sims-vm/src/routing/search.rs:1-10,78-81` |
| `portals.rs` | 0x46390f cost cache: 10001/1001/1101 sentinels, partner 0x4636ae, attempt and fail | ported, except the 0x46387c invalidation (#3) | `crates/sims-vm/src/routing/portals.rs:24-63,373-420` |
| `lot_route.rs`, `lot_route/{search,geometry,obstacle,candidate,seat,live}.rs` | 0x4818af room rectangles and hard-prefix inflate 7; 0x482608 dynamic obstacles; 0x4823fa planner policy (bounded/unbounded pass, 0x482e70 result 3); 0x482861 goals | ported | `crates/sims-vm/src/routing/{rooms,planner}.rs` (`planner.rs:1-37` cites `lot_route/search.rs` and `geometry.rs`) |
| `lot_route/{completion,portal,departure,retry,movement,animator,shoo}.rs` | 0x45ad63 state machine: portal dispatch 0x45c7c2, completion states 7 and 8, stand dispatch | mostly ported. Kept as reference for the branches `go_to_slot.rs` still traps on (0x45b285, 0x45b2f7, 0x45af5b, 0x45c437) | `crates/sims-vm/src/primitives/go_to_slot.rs` |
| `lot_route/refresh.rs` | 0x444805/0x45eddc/0x46387c refresh invalidation; 0x45ad63 resume (state 2 forgets the portal, state 3 stops walking) | **to-mine (#3)** | `go_to_slot.rs:500-508` traps at 0x45aea5; `selection.rs:417` "Not modeled" |
| `contention.rs`, `lot_snap/shoo.rs` | 0x45ba28/0x45bf2e corridor shoo, 0x45bd5e placement shoo, 0x4817e9 gate, 0x589ead interrupt gate, snap field 52 | ported | `crates/sims-vm/src/primitives/in_way.rs:1-60`, `primitives/snap.rs:97,546` |
| `placement.rs`, `lot_find_location.rs`, `lot_acceptance(+/)`, `spatial.rs` | 0x4347f0 spiral, mode 5 random origin (0x43494a), 0x433311/0x4329c1/0x4337f6/0x422abb acceptance, 0x433d16 footprint | ported | `crates/sims-vm/src/placement/find.rs:12-17`, `primitives/find_location.rs:5-60,201-260` |
| `lot_placement(+/)`, `lot_links(+/)`, `lot_adjacency.rs`, `lot_loading.rs`, `lot_dynamic(+/)`, `lot_transfer.rs`, `transfer.rs`, `lot_snap.rs` | SetObject/RemoveFromLot ordering, physical slot links, 0x433612 seat facing, post-load spatial pass, 0x4235ee dynamic groups, Grab/Drop/Drop Onto, Snap 0x454976 | ported | `crates/sims-vm/src/placement.rs:26-30`, `placement/dynamic.rs`, `primitives/{grab,drop,drop_onto,snap}.rs` |
| `lot_ground.rs` | 0x48cbec "Go Here" ground destination: marker 0x7c4, 0x4347f0 mode 1 in the same room with occupied tiles allowed, then accept and place; 0x44123f cleanup | **to-mine (#4)** | `crates/sims-game/src/game.rs:5463` (`open_menu`) handles object presses only; `docs/roadmap.md:2469` shows the original opening a Go Here pie |
| `clock.rs` | 0x46d345 calendar: 2 s per tick, 0xdb/0xdc/0xf8/0xdd signals, 30-day months (0x473544), time-of-day periods, budget forward-copy | ported | `crates/sims-vm/src/tick.rs:13-23,564-629`, `daily.rs:1-60,357-392` |
| `house_statistics.rs` | HOUS 1 usage pairs, 0x4165e3 sample, 0x41637f shift | ported | `crates/sims-sim/src/world.rs:694-724`, `daily.rs:384` |
| `scheduler.rs`, `household.rs`, `lib.rs`, `fixture_driver.rs`, `snapshot.rs` | native registry cursor, budget suspension, laboratory Simulation host, JSON snapshot | dropped (continuations, fixture glue), superseded by `tick.rs` and `crates/sims-sim/src/snapshot.rs` | salvage-audit:187-188; vision:22-25 |
| `interaction.rs`, `immediate.rs`, `cancellation.rs`, `action_execution.rs`, `service_observation(+/)`, `ui_wait.rs` | laboratory chair-room host, frame-arena callbacks, cancel Function 4 (0x422d46→0x4749b5), observation sinks, 0x4764fe | dropped (laboratory and observation tooling); cancel and the 0x4764fe callbacks are ported | `crates/sims-vm/src/cancel.rs`, `selection.rs:431-435` |
| `removal.rs`, `removal/*`, `lot_removal(+/)`, `lot_clear.rs`, `lot_reset(+/)`, `person_reset(+/)` | 0x440eeb delete, 0x44123f remove-now, 0x4412d1 drain, 0x44279a clear, 0x431fbe reset, 0x45e791 person reset, 0x447d8b unregister | ported (three cases); graphic and character retention dropped | `crates/sims-vm/src/removal.rs:1-37`, `forget.rs`, `visits.rs:39` |
| `burn.rs`, `burn/*` | 0x44d96f Burn spread, person rectangle widened by 2 (0x44de25), fire placement | ported | `crates/sims-vm/src/primitives/burn.rs:30-50,275-300` |
| `lot_controllers.rs` | 0x40726a comeandsee: excluded lots, 0x4074df existing; 0x43520d elapsed minutes, 0x473574 day number (1900 epoch, 360-day year) | ported, except the exact calendar arithmetic (**#13**) | `crates/sims-vm/src/buy.rs:307-400` (uses equality of five words) |
| `create.rs`, `lot_creation(+/)`, `person_creation(+tests)` | 0x440b2d factory, group factory, 0x44e0c8 Create Object, face-as-stack 0x44e64d, person factory | ported (the 0x44e70d image-cache part is presentation) | `crates/sims-vm/src/{factory,group_factory,person_factory}.rs`, `primitives/create.rs:6` |
| `lot_selection(+/)`, `lot_pickup(+/)` | 0x445449 select, indicator 0x44664c; RemoveFromLot 0x435751/0x422ff5 | ported | `crates/sims-vm/src/selection.rs`, `buy.rs:1842,2448` |
| `lot_cursor(+/)` | Tile Cursor 0x4215e6: move, rotate, release, reset, permission | ported as Buy commands. Cheat and refusal-code discrepancy: **#12** | `crates/sims-vm/src/buy.rs:774-918,2273-2336`; decisions 20261121-buy-mode-* |
| `edit_history(+/)` | UndoManager 0x493259…, Buy/Move/Sell undoables, budget flush | ported as commands; generic engine dropped | `buy.rs:95`, `boot.rs:2991`, `build.rs:2018-2221` |
| `object_catalog.rs`, `object_catalog/{definition,iff,products,categories}` | 0x43b89c registration, OBJD normalization, buy eligibility 0x4cd149, category masks | ported | `crates/sims-content/src/catalog.rs`, `sims-vm/src/program.rs:1794`, `sims-ui/src/buy.rs:492-905` |
| `object_catalog/resources.rs` | object resource cache keyed by basename plus extension, truncated to 31 bytes, case-insensitive | **to-mine (verify, #16)** | `crates/sims-content/src/catalog.rs:338` keys by basename |
| `object_catalog/move_tool(+/)` | commit 0x486ada, sell 0x487e18, teardown, feedback; tool fields; hover 0x486d95 | commit, sell and feedback ported. Tool classification (**#14**) and hover flags (**#15**) are to-mine | `buy.rs:1752`, `boot.rs:2653`, `sims-ui/src/build.rs:804` |
| `object_catalog/iff/write.rs`, `{access,directory,composite,sprite_pool}` | IFFResFile3 writeback (0x41cf32 quirk); Win32 streams | dropped (vision:26 says no original save format; Win32) | `crates/sims-formats/src/iff_write.rs` holds the ResFile2 writer |
| `object_scan(+/)` | 0x43c609 phase order, skip `*.dir` | ported | `crates/sims-content/src/install.rs:113`, `index.rs:537` |
| `person_notification(+/)` | 0x4a69aa word-1 dispatch; display cache (`now - cached < 100`, 0x437e9c), colours, sting_char_winner; currency formatter 0x6007df | state and "-$n" negative formatting ported (#484, `sims_ui::currency`); display and sound are **to-mine (#9)** | `crates/sims-vm/src/tick.rs:931-939,1126`; `sims-ui/src/household.rs:770-789` |
| `pose/movement(+/path,steering,projection,route,calibration)` | walk step under PC=24 (0x58ede3), heading, steering, snap 0x48313e, states 0-4, 6, 9-11 | ported | `crates/sims-vm/src/animator.rs:617-654,968-1090,1879,2491,2653` |
| `pose/movement.rs` states 5, 7, 8, 12 | sidestep, stop while walking, flag state, wait for an interruptible clip; constants 0x6d07b0/0x6d07bc/0x6d07e0 | ported (#485) | `crates/sims-vm/src/animator.rs`; ADR `20261126-walk-animator-states-and-footsteps.md` |
| `pose/{clock,update,layer,events,posture,ordinary,animator,command,accessory}`, `carry.rs` | layer clock and fade 0x597c4c, event window, posture 0x590430, carry clip 0x26, Animate Sim 0x58950b, dress 0x58d88f | ported | `crates/sims-sim/src/animator.rs:565-790`; `sims-vm/src/animator.rs:14-37`, `primitives/animate.rs`, `change_suit.rs` |
| `pose/properties.rs` sound/footstep, `properties/surface.rs` | footstep gating, surface precedence (0x5890a8), names, selected/deselected sound | ported (#485) | `crates/sims-vm/src/footsteps.rs`, `clip_events.rs` |
| `pose/sampling.rs` masking, `layer.rs:208` | bone-priority and translation-policy masking, blend marker 0x595947, layer order 0x5991fd | **to-mine, render (#5)** | `crates/sims-people/src/person.rs:257-272` applies layers unmasked |
| `pose.rs`, `pose/{transform,sampling(frame),skeleton}`, `head_look(+/rig)`, `turn.rs`, `native_turn.rs`, `reach.rs`, `lot_reach(+/)`, `portrait_person.rs`, `person_update.rs`, `character_profile.rs` | quaternion blend, model matrix 0x592132, head look 0x590768/0x5904d2, Look Towards 0x45a98e, Reach 0x45c9ba, portrait owner 0x4e9652, person tick 0x45cc8c | ported. Two small items to mine: wiggle noise values (#18) and body scale (#6) | `crates/sims-sim/src/{pose,head}.rs`; `sims-vm/src/{head.rs,primitives/reach.rs,primitives/look_towards.rs,tick.rs:900-940}` |
| `pose/portrait(+/)`, `pose/clothing(+/)`, `appearance_*` | head crop and border; clothing patterns, outfit selection 0x58a178; avatar search paths | ported, with residue: portrait light (#17), path-normalization residue (#19) | `crates/sims-people/src/{caf_preview,suit,appearance}.rs`, `sims-vm/src/outfits.rs`, `sims-content/src/people.rs` |
| `pose/{mesh,vertices,geometry,model_*,graphic_*,live,refresh,initialize,construction,destruction,reset}`, `character*.rs`, `animation.rs`, `locomotion.rs` | D3D mesh and material lifetimes, graphic owners, resumable character services; the prototype's made-up 30 fps cursors | dropped (render infrastructure, continuations, not native) | salvage-audit:186 (pose subset only); vision:22-25 |
| `lot_presenter` projection, depth, cutaway, walls, sprites, lights, grass, scroll, centering | 0x4a955d/0x4a9dc7 projection, 0x4ca5af depth table, 0x468203 room masks, wall caps 0x53d2fc, blitters, light pools 0x4a3813, grass 0x49dacd | ported | `crates/sims-render/src/{camera,cutaway,depth,caps,canvas,pools,light,shadows,grass}.rs`; `sims-game/src/game.rs:13266-13393` |
| `lot_presenter/walls/rotation.rs`, `level_geometry` (wall order) | 0x5404b5 re-sort, 0x5409ab lower_bound insert, 0x53f1db heading | ported (#486) | `crates/sims-render` wall runs in graph order (`0x540619`) |
| `lot_presenter/motion.rs`, `camera_command.rs` setters, `ground.rs` | linear glide 0x4c9a27; zoom-out floor 2 on house 99; 0x4af029 ground-under-tile rule | ported (#486) | `Zoom::zoom_out_on`; the ground rule hides ground only under floored water neighbours (§3 item 11) |
| `lot_presenter` plumbing (viewport, redraw, open, lifecycle, person_refresh, physical_next, row_tail stages, motion/frame) | dirty rectangles, F6 ownership, native resume points (66 unmatched addresses) | dropped | salvage-audit:189-191 |
| `lot_rooms.rs`, `lot_flags.rs`, `lot_walls.rs`, `lot_geometry.rs`, `architecture.rs`, `geometry.rs` | 0x479d9a partition, exterior and build-support flags, 0x476dbc wall collision | ported (the prototype's fixture topology is dropped) | `crates/sims-sim/src/rooms.rs:1-24,67`, `load.rs:249-350` |
| `lot_environment(+/)` | 0x4665d1/0x467980 room scores, light byte, lamp pairs 0x4b5037, cap of 6 lamps | ported (the 0x4b51ae removal order is recomputed each frame) | `crates/sims-sim/src/rooms.rs:979`, `sims-render/src/lot.rs:115-126,531` |
| `house_valuation(+/)` | 0x444cb9 features, 0x43219a depreciation, 0x4354d5; 0x47b006 architecture value | partly ported (the load path computes the architecture value in `crates/sims-vm/src/lifecycle.rs:524-680`); the day-turn architecture value is **to-mine (#6a)** | `crates/sims-vm/src/daily.rs:400-436` ("known gap" at :435) |
| `saved_world`, `saved_geometry`, `saved_matrix.rs`, `saved_types.rs`, `surface_remap(+/)` | 0x478599 arrays, 0x431005 reads, 0x43dd76 type linking, 0x4a0e3b/0x4c5d35 surface remap | ported (the 0x478909 light reset is presentation) | `crates/sims-sim/src/load.rs:590-760`, `sims-vm/src/load.rs:280` |
| `native_grass(+/)` | grass generator for a fresh or resized lot: seed 0x1de48a, `rand%160-32` grid, refine passes, RNG restored | **to-mine, low (#20)**. Needed only if new lots are created | nothing in `crates/sims-sim/src/grass.rs` |
| `native_world(+/)`, `native_world_children(+/)`, `native_matrix(+/)`, `native_wall_io(+/)`, `native_wall_vertices(+/)` | resource manager, path and strings, startup naming, matrix resize, wall service write tape. Real parts: channel tables, rotation tables, room-map fill, wall service | real rules ported (`sims-render/src/light.rs:12-25`, `rooms.rs:67`, `load.rs:249-350`); the rest dropped | salvage-audit:192-193 |
| `native_application(+/)`, `native_system(+/)`, `native_game(+/)` | Win32 app bootstrap: COM, mutex, FPU, critical sections, provider factories, constructor images | dropped | vision:22-23; salvage-audit:192 |
| `*_tests.rs` (character_*, turn, person_creation, script_direction), in-src `tests.rs` and `lot_runtime(+/)` | continuation and oracle tests; `lot_runtime/tests.rs` (7.5k) is load-to-scheduler handoff | dropped (private fixtures or continuations) | vision:22-25; fixtures absent |
| `tests/` (206 files) | 176 use absent `fixtures/*.json` or `SIMS_*` reports. About 30 are synthetic: room, flags and route rules already have equivalents; edit and buy tests encode undo bookkeeping | dropped or already covered | `crates/sims-sim/tests/integration/rooms.rs`, `crates/sims-vm/src/routing/rooms.rs` |
| `examples/compare_native_{clock,routing}.rs` | compare against private oracle reports | dropped (evidence tooling) | vision:22-25 |

### 3. Still-to-mine backlog

Items are ordered by gameplay impact. Line numbers refer to the legacy files.

**Simulation (`sims-vm` / `sims-sim`)**

1. **Walk animator states 5, 7, 8 and 12, and the short step.** *Ported* (#485, [ADR](decisions/20261126-walk-animator-states-and-footsteps.md)).
   - Source: `legacy/crates/sims-sim/src/pose/movement.rs:420-470, 501-537, 572-652, 720-745` and `pose/movement/path.rs` (`prepare_stop`).
   - What it covers:
     - State 7 stops while walking: clip 19, heading and position eased by phase, and the reason mapped to −1/2/3.
     - The sidestep octant table is [15,13,11,12,14,16,18,17], used within 0.5 to 3.1 of the target. Pets scale distances by 1/12 and 0.2.
     - Constants 0x6d07b0, 0x6d07bc and 0x6d07e0.
   - Destination: `crates/sims-vm/src/animator.rs`, which ran these states through the 0x58faa2 jump table since #485 (it trapped at 0x58faa2 and 0x58df1e before).
2. **Footstep and clip-sound selection.** *Ported* (#485): `crates/sims-vm/src/footsteps.rs` and `clip_events.rs`.
   - Source: `pose/properties.rs:154-236, 264-286` and `pose/properties/surface.rs`.
   - What it covers:
     - Gating: x ≠ −999, the event's ±1/±2 sign matched to the clip's direction, and person flags 84&1, 68&1 and 74&8 all clear.
     - Surface precedence: water, then the GUID ranks, then pool, then the floor-category byte.
     - Names `footstep_{hard,medium,soft,terrain}` plus `_noshoe`; lot 99 → soft.
     - `selectedsound` / `deselectedsound`.
   - Destination: `crates/sims-vm/src/clip_events.rs` and `sounds.rs`.
3. **Route-cache invalidation on house refresh.**
   - Source: `lot_route/refresh.rs` and `portals.rs:98` (`invalidate`).
   - What it covers:
     - 0x444805 clears every person's context byte +0x78.
     - 0x46387c refills portal costs with 10001 and keeps the columns allocated.
     - On resume, 0x45ad63 sets the byte again. In state 2 it forgets the last portal (see `lot_route/portal.rs:341-384`); in state 3 it stops walking.
   - Destination: `crates/sims-vm/src/primitives/go_to_slot.rs:500-508`, `routing/portals.rs` and `selection.rs:417`.
4. **"Go Here" ground click.**
   - Source: `lot_ground.rs`.
   - What it covers:
     - 0x48cbec makes a marker of type 0x7c4.
     - Spiral search 0x4347f0, mode 1, in the same room, with occupied tiles allowed.
     - Floor acceptance, then SetObject.
     - 0x44123f cleanup on failure or cancel.
   - Destination: `crates/sims-vm/src/orders.rs` plus `placement/find.rs`, and the press path in `crates/sims-game` (`game.rs` `open_menu`). Check with the input track.
5. **Layer masking when posing.**
   - Source: `pose/sampling.rs:160-230` and `pose/layer.rs:208`.
   - What it covers: a bone is skipped when its priority exceeds the layer's; translation policy −1 and the blend marker 0x595947; layers ordered by priority with stable ties (0x5991fd, 0x597c1d).
   - Destination: `crates/sims-people/src/{animation,person}.rs`.
6. **Architecture value and body scale.**
   - (a) Architecture value, stored at simulator +0xa4. The lot-load path already computes it (`crates/sims-vm/src/lifecycle.rs:524-680`, writing `budget.architecture_value`). What remains is the day turn: `daily.rs` keeps the saved value ("known gap" at :435), and a lot where only walls count for global 22 fails because wall resolution (0x4795c5) is not ported there.
     - Source: `house_valuation.rs:11-30`.
     - Rule (0x47b006): wall cost is halved once; split-floor halves are truncated separately; sums wrap as i32; visit order is floor, Y, X.
     - Destination: `crates/sims-vm/src/daily.rs:423-436`.
   - (b) Body scale.
     - Source: `person_update.rs:165-172` and `pose/model.rs:206-230`.
     - Rule: word 85 × 0.01, every 10 ticks, applied when 0.4 < v < 10 (0x5a0932).
     - Destination: `crates/sims-people`.

**Presentation (`sims-render` / `sims-game` / `sims-ui`)**

7. **Original wall draw order.** *Ported* (#486, [ADR](decisions/20261126-render-censor-held-order-walls-glide.md)).
   - Source: `lot_presenter/walls/rotation.rs:25-40,166-179`, `lot_presenter/walls/construction.rs:232-237` and `lot_presenter/level_geometry.rs`.
   - Rule: records sorted on (x+y, x−y) with the original's unstable sort; 0x5409ab lower_bound insert; the vectors are re-sorted even at rotation 0; heading from 0x53f1db.
   - Destination: `crates/sims-render/src/scene.rs:2376`.
8. **Linear camera glide.** *Ported* (#486): 250 ms, `target - slope * remaining` in single precision, rounded half away from zero.
   - Source: `lot_presenter/motion.rs:32-135` and `centering.rs:15-20,203`.
   - Rule (0x4c9a27): target − slope·remaining, rounded. Duration is the value from 0x6c7788 or distance/speed + 0.5. Style 0 jumps.
   - Destination: `crates/sims-game/src/game.rs:172-205`, replacing the cubic ease.
9. **Floating text over a person.**
   - Source: `person_notification/display.rs:305` and `person_notification.rs:24-28,100-125`.
   - What it covers: lifetime `now − cached < 100`; yellow (255,203,23) and magenta (255,0,255) styles; the `sting_char_winner` sound; the 0x4a698d label path.
   - Destination: `crates/sims-vm/src/tick.rs:1126` and `sounds.rs`, plus render and game.
10. **Currency formatter.** *Ported* (#484, `sims_ui::currency`, [ADR](decisions/20261126-ui-buy-labels-family-export-dialog-presentation-currency.md)).
    - Source: `person_notification/currency.rs`.
    - Rule (0x6007df): negatives format as "-$n" in language 1, which settles the "unverified" note at `sims-ui/src/household.rs:771`. The per-language separator and symbol table is needed only for localisation.
    - Destination: one shared formatter in `crates/sims-ui`.
11. **Small camera and ground rules.** *Ported* (#486).
    - Zoom-out stops at 2 on house 99. Source: `lot_presenter/camera_command.rs:100-107`. Destination: `crates/sims-render/src/camera.rs:65-78`.
    - The ground under a level-1 floor is hidden only when the tile and its four neighbours are floored and all four neighbours are water; neighbour clamps use the active size. (This entry used to say the ground is drawn when "a neighbour is water", which inverts the water test, #486.) The global enable byte `0x6c724a` is not ported, so the rule is always on. Source: `lot_presenter/ground.rs:3-45` (0x4af029). Destination: `crates/sims-render/src/scene.rs:1767`.

**Buy/Build and catalog (`sims-vm` / `sims-content`)**

12. **Cheat 0x6e4ad4 and refusal codes.**
    - Source: `lot_cursor/permission.rs:5-60,154`.
    - What it covers:
      - Legacy reads 0x6e4ad4 as the `move_objects` cheat, which bypasses the pickup and delete checks.
      - Refusal codes 0x25, 0x26 and 0x27.
      - Children are visited last slot first.
    - Destination: `crates/sims-vm/src/buy.rs:774-918` and `placement/find.rs:610`, where the cheat is currently "taken to be 0".
13. **Calendar arithmetic.**
    - Source: `lot_controllers.rs:28-66`.
    - Rule: 0x473574 counts days from a 1900 epoch with 360-day years; years before 1900 or bad months give 0. 0x43520d computes elapsed minutes with borrows.
    - Destination: `crates/sims-vm/src/buy.rs:383-392`, which compares five words for equality.
14. **Build-tool classification.**
    - Source: `object_catalog/move_tool.rs:120-230` and `move_tool/selected.rs`.
    - Rule: tool mode = group+3; flags; door/window `special`; stairs `subtype`; door and window camera offsets from 0x487241; the first registered master wins (0x43ae0a).
    - Destination: `crates/sims-vm/src/{buy,build}.rs`.
15. **Hand-tool hover flags.**
    - Source: `object_catalog/move_tool/hover.rs`.
    - Rule (0x486d95): clear `&0xe` on the old object, then set `(f&0x1e)|(0x11 or 1)` on the new one.
    - Destination: `crates/sims-vm` (object +0x114), read by render.
16. **Object resource-cache identity (verify).**
    - Source: `object_catalog/resources.rs:1-80`.
    - Rule: the key is basename plus extension, truncated to 31 bytes, compared case-insensitively. Two IFFs with the same name in different directories share one entry.
    - Destination: `crates/sims-content/src/catalog.rs:338`.

**Low priority**

17. **Portrait light direction.** Turned by π/9 (0x4e7b24..7bb0). Source: `pose/portrait/view.rs:193-200`. Destination: `crates/sims-people/src/caf_preview.rs`.
18. **Bone wiggle noise values.** 0x595d9c and 0x595de8, used only if wiggle is drawn. Source: `pose/skeleton.rs:35-60`. Destination: `crates/sims-people/src/rig.rs`.
19. **Content-path residue.** Semicolon path normalization (0x5c2f41, 0x5c3136), the wildcard re-open retry (0x5c1f62), and skeleton/animation name-map order (0x5a1ded..1ef4). Source: `pose/clothing/{catalog_paths,catalog_open,name_map}.rs`. Destination: `crates/sims-content`.
20. **Fresh-lot grass generator.** Seed 0x1de48a, a 16-step `rand%160−32` grid, refine passes (8,80)…(1,10), and the RNG restored afterwards. Needed only if new or resized lots are supported. Source: `native_grass.rs:43-110`. Destination: `crates/sims-sim/src/grass.rs`.

**Reference only (no new rule, but read it when the trap is hit):**

- `lot_route/{completion,portal,departure,retry,movement}.rs`, for the branches `go_to_slot.rs` still traps on: 0x45b285, 0x45b2f7, 0x45afc4, 0x45af5b and 0x45c437.
- `removal/{cursor,entry}.rs`, for the 0x440eeb branches `crates/sims-vm/src/removal.rs:628` does not yet support (type-9 cursor deletion).

**Checks, not ports:**

- `lot_presenter/floor_prep.rs:80-100`: does a diagonal half with style 0 and pattern 0 take the tile's floor? Compare with `sims-render/src/lot.rs:50`.
- The comment at `crates/sims-sim/src/world.rs:703` saying usage is "not ported" is stale.
