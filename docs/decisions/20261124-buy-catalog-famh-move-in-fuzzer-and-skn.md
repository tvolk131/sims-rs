# Buy catalog parity, `.FAM` FAMh at move-in, the state fuzzer, and the SKN empty line

*Decided 2026-11-24.*

- **Status:** Accepted
- **Scope:** `crates/sims-content/src/buy_catalog.rs`, `crates/sims-sim/src/{neighborhood_save,neighborhood_store,patch_import}.rs`, `crates/sims-sim/tests/state_fuzzer.rs`, `crates/sims-formats/src/avatar/text.rs`, removal of `legacy/crates/sims-formats`
- **Track:** Track 4

## Context

The legacy mining audit (`docs/legacy-mining-audit.md`) left these items open:

- `sims-content` items 1–3: which objects the Buy catalog lists, the name and description it shows, and the category button labels;
- the FAMh a family brings from a `.FAM` file when it moves in;
- `sims-runtime` item 1: the exploration fuzzer, the salvage audit's one PORT item;
- `sims-formats`: whether an SKN texture line may be empty, and two test groups to re-express before the crate goes.

The legacy code had rules for each, but several of them were guesses. Every rule below was checked against the 1.91 disassembly. Where the legacy rule differed, the disassembly wins, and the difference is listed.

All tests are synthetic. The install-gated catalog test prints counts only.

## Decision 1: Buy catalog eligibility follows the builder's order

`BuyDefinition::eligibility` follows the catalog builder (0x4cd83b–0x4cd86b) in this order:

1. **Excluded:** a tile that is not its group's master (0x4575b8: group ≠ 0 and subindex ≠ −1), or a person (0x4575d2: kind 2).
2. **Listed:** a master, or a definition with a nonzero graphics word +8 (0x457588).
3. **Otherwise listed only if its interaction table is nonempty** (0x4575a1: the table exists and its count at +4 is nonzero).

The TTAB id (`interaction_table`, 0x45726a):

- An id ≥ 0 is used as is.
- −1 takes the master's word (0x45728d → 0x43ae0a). The master is the definition itself when its group is 0. Otherwise it is the first definition registered in the same file with the same group that is a master (0x5c18e3). With no master, the original dereferences null; the port answers None.
- Any other negative id is 0, and 0x4569a3 rejects ids ≤ 0.

The table is looked up in the object's own file first (0x416920), then in its semi-global (0x416924). A table that loads but is empty still hides the semi-global's. The global file is never read.

## Decision 2: catalog text reads CATS name second and CTSS in the game language only

`CatalogText::read` follows 0x457d3b and 0x4061e3:

- **CATS:** the first C-string is the description (+0xc). A second string, if it starts before the chunk ends, is the name (+0). The legacy code had these the other way round.
- **Otherwise CTSS** (0x470256 with language = game language − 1), loaded by 0x470ca7(0, 0). That load files the current language only, with no fallback per entry. The legacy code fell back to English per string. The name is string 1 and the description string 2 (0x471411, 1-based).
- **People** (0x4062c1): if the name is empty, the table is loaded again for all languages (0x470ca7(1, 0)), and 0x47145f probes internal languages 0–19 for the first slot-1 string, even an empty one.
- **An empty name** becomes `*` (0x6c12fc) followed by the OBJD chunk label as a C-string (0x457daf–0x457de2).

The string-table filer (0x470ca7 and 0x46feac are the same logic) is ported privately as `Filed`:

- the format word is byte-swapped unless flag 0x10 is set;
- formats −1 and −2 file into the current language and language 0;
- format −3 files by stored language − 1 (a stored 0 means current), restarting its run index at each language change. 0x471343 inserts at the clamped 1-based place, so a later run goes in front.
- A positive word is a Pascal count from +2, with 0xa1 → `o` and 0xd5 → `'`.

## Decision 3: Buy category labels come from UIText.iff with the original's remap

`BuyLabels` reads `GameData/UIText.iff` through 0x4fff1b → 0x47028b → 0x46feac(0, 1). The English fallback is for the whole table: English is used only when the game language has no strings at all.

- STR# 150: 56 strings, and roots[c][i] is place c·8 + i + 1 (0x6f2ff0).
- STR# 200–207: four function labels each (0x6f2e68 + row·0x30).
- STR# 210: entries 2, 3, 4, 5 (0x6f2e58, 0x6f2e48, 0x6f2e38, 0x6f2e28).

`BuyLabels::label` follows 0x506765:

- **Catalog 1 inside a subcatalog:** columns 0–3 are the row's function labels, and columns 4–7 are STR# 210 entries 4, 5, 2 and 3.
- **Any other catalog inside a subcatalog:** roots[1].
- **Subcatalog 0:** catalogs 0–4 use their own roots row, and catalogs 5 and 6 use roots[5]. Catalog 6 button 2 is roots[5][4] (0x6f3200), and button 3 is roots[4][2] (0x6f3188).
- **Hidden buttons:** catalogs 2–6 at subcatalog 0 hide columns 4–6 (`shown`).

`Catalog::buy_entries` puts the three decisions together over an install. Over the complete collection it lists 1,835 of 5,289 definitions, and 175 of those take the `*` label name: 171 have no CTSS, and 4 have a CTSS with no English name. `sims-ui` and `sims-game` keep their current catalog code; switching them over belongs to their tracks.

**Open:** whether the file's string lookup (vtable +0x38) also searches the semi-global was not traced. The port reads the object's own file, as `sims-game`'s `catalog_strings` does.

## Decision 4: a family moving in from a `.FAM` file brings that file's FAMh

The import hands the `.FAM` file to the family (+0x140, 0x4291e6). When the neighborhood is saved, 0x4107a8 copies the family's FAMh from that file. Only a family with no file falls back to its stored history (+0x144, 0x4107c7–0x4107cf). So a `.FAM` file without a FAMh leaves the family without one, even if the ID had one before.

- `SaveInputs::move_in_family(family, history)` sets or clears the FAMh. It also drops the name and stray name byte read for the ID, because they belonged to whoever had the ID before. FAMs is then written from the stored name.
- `SaveInputs::renumber_family(from, to)` moves all three entries, as a family renumbered in memory keeps its file and record (0x42c1ad renumbers family 4000). `to`'s own entries are replaced.
- `NeighborhoodStore::save_characters_neighborhood_and_save_inputs_checked` applies the change in the same transaction as the record. The change runs before the store forgets the save inputs of families the commit drops.
- The Patch import calls `Opened::update_save_inputs` to apply both.

## Decision 5: the exploration fuzzer's design, ported for the state layer

`crates/sims-sim/tests/state_fuzzer.rs` carries over the legacy fuzzer's design (`legacy-final:crates/sims-runtime/src/exploration.rs`):

- a SplitMix64 stream with weighted command choice and bounded draws by rejection sampling;
- the whole command list drawn before it runs;
- a per-command trace compared at the first differing command;
- warm-fork and cold-restore branches at several command boundaries, each required;
- panic capture;
- delta-debugging of a failing list, reported with its seed.

It runs on `sims-sim` state alone, because `sims-sim` cannot depend on `sims-vm`:

- **World.** Seeded commands draw from the three random streams, set globals, add, edit and remove objects, push frames, register types, set scheduler flags and countdowns, and queue indicator refreshes. At up to three boundaries, the run branches three ways: a warm fork (a clone sharing copy-on-write storage), a cold restore through `encode_snapshot`, and, for some seeds, a restore through a REDB `Snapshots` file. Each branch runs the rest at once and must trace what a never-forked reference traced. The run the branches came from must still match the reference, which catches a fork that writes through to its parent.
  - The trace holds each command's answer, `World::hash` and the indicator refreshes, which the hash leaves out.
  - The invariants checked are: `hash` is SHA-256 of `encode`; `encode`/`decode` and `encode_snapshot`/`decode_snapshot` read back the same bytes; and the plain decoder still reads a snapshot payload.
- **Records.** Randomized `StoredNeighborhood` and `SaveInputs` values round-trip through `encode`/`decode` (and `decode_strict`). A cut encoding is refused, and `neighborhood_iff` depends only on the record and the inputs.
- **The store.** Random operations run against `NeighborhoodStore` and a model of its rules: plain, checked (current and stale) and character-batch commits with move-in and renumbering, input updates, character files, base houses, and reopening the file. The rules modelled are:
  - a new record gets a fresh instance and revision 0;
  - each commit advances the revision;
  - a dropped family's save inputs are forgotten;
  - character and house writes bump the revision;
  - stale and repeated-name commits change nothing.

  After every operation, the record, the inputs, the files and the saved `Neighborhood.iff` must match the model.

The tests take about 5 s in release. Deliberately breaking the model (not forgetting dropped families) or the restore (decoding without the refreshes) makes them fail, with a shrunk list.

The tick-level driver (net commands, `Lot::save`/restore, ticks) stays on the audit's backlog for a `sims-vm` test. It can copy the sampler and `check_or_shrink`.

## Decision 6: an empty SKN texture line fails, as in the original

0x59a629 reads the name, the texture and each bone name with one line loop (0x59a644, 0x59a6a9, 0x59a733: getline of up to 0x400 bytes), and that loop skips empty lines (0x59a676, 0x59a6db, 0x59a769). An empty texture line is therefore skipped, and the bone count is taken as the texture. The count is then read from the first bone name, and the read fails (0x59a921).

`read_skn` now reads the texture with the same line-skipping reader. Three EP7 accessory meshes have an empty texture line; they now fail to load, as they do in the original, instead of loading untextured.

The catalog short-read tests and the text-avatar rejection tests were re-expressed in `crates/sims-formats/tests/avatar_codecs/`, and `legacy/crates/sims-formats` was removed. `legacy/README.md` drops the crate from its list.

## Consequences

- `sims-content` gains an additive public API (`BuyCatalog`, `BuyDefinition`, `BuyEligibility`, `BuyEntry`, `BuyLabels`, `CatalogText`, `has_interactions`, `master_of`). No existing signature changes.
- `sims-sim` gains `SaveInputs::move_in_family` and `renumber_family`, the `SaveInputsChange` alias, the checked commit with a save-inputs change, and `Opened::update_save_inputs`.
- CI gains a deterministic fuzzer for snapshots and the neighborhood store. A failure prints its seed and a shrunk command list.
- The audit marks `sims-content` items 1–3 and the `sims-formats` items as done, and `sims-runtime` item 1 as partly ported.

## What would reopen this

- A harness capture of the Buy catalog that lists, names or labels an object differently from `buy_entries`. The first place to look is the untraced string lookup (**unverified**: own file only).
- The original saving a FAMh for a moved-in family whose `.FAM` file has none.
- An EP7 accessory with an empty texture line drawing in the original.
