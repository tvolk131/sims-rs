# `.FAM` FAMh at move-in in `sims-vm`, a creation-depth guard, and small cleanups

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-vm/src/{caf,neighborhood_open,factory,group_factory,person_factory,interpreter,lib}.rs`
  - `crates/sims-vm/src/neighborhood_open/pools.rs`, `crates/sims-vm/src/primitives/{create,ui_effect,dialog}.rs`
  - `crates/sims-vm/tests/{integration/caf.rs,integration/boot.rs}`, `crates/sims-vm/tests/interpreter/{factory,create_person,ui_effect}.rs`
  - removal of `legacy/fixtures/suites/`
  - `legacy/README.md`, `docs/repository-workflow.md`, and a comment in `crates/sims-sim/src/pose.rs`
- **Track:** Track 2

## Context

`20261124-buy-catalog-famh-move-in-fuzzer-and-skn.md` (Decision 4) added the `sims-sim` API that records the FAMh a family brings from its `.FAM` file:
- `SaveInputs::move_in_family`;
- the checked commit with a save-inputs change;
- `patch_import::Opened::update_save_inputs`.

Neither `sims-vm` import path called them yet, so an imported family was saved without its FAMh. These are the `.FAM` Import button (`caf::import_family_fam`) and the `Patch` open (`neighborhood_open::open_neighborhood`).

`20261124-antics-test-runner.md` notes that the factory runs a new object's Init and first turn inside the creation with no depth limit. An object whose Init or Main makes another of its own kind therefore nests creations until the stack overflows. The runner works around that with a 2 GiB thread, and asked for a guard in the factory as a follow-up.

## Decision 1: the `.FAM` Import button commits the file's FAMh with the household

`import_family_fam` now commits through `save_characters_neighborhood_and_save_inputs_checked`. The change it passes is `move_in_family(family_id, parsed.history)`, where `family_id` is the ID the family is given, which differs from the file's when that one is taken.

The import hands the file to the family (+0x140, 0x4291e6), and the save copies the family's FAMh from that file (0x4107a8). It falls back to the stored record (+0x144) only when there is no file. So:
- A file with a FAMh gives the family that payload, byte for byte.
- A file without one leaves the family none, even if an earlier holder of the ID left one.

In both cases, the name read for the ID and its stray byte are dropped, so FAMs is written from the stored name.

The change runs in the same transaction as the household. A stale-revision or instance mismatch therefore leaves the save inputs untouched as well.

Create-a-Family (`commit_family`) is unchanged. It makes a family with no file and no record, which the store's existing inputs already save without a FAMh.

## Decision 2: the `Patch` open returns the save inputs of the save after the import

The original saves `Neighborhood.iff` after the import (0x4252cc). That save is still not ported: the opened neighborhood is returned in memory. `neighborhood_open::Opened` now also carries `save_inputs`, the inputs that save would write, computed in three steps:
1. Read them from the same `Neighborhood.iff` the open reads (`SaveInputs::from_file`), kept to the families it holds, as the store's import does.
2. Apply the import's changes (`patch_import::Opened::update_save_inputs`): each imported family takes its file's FAMh or none, and the renumbered family 4000 keeps its own under its new ID.
3. Carry them through the pools. When the Townie or tourist wrapper finds a household at one of its reserved IDs (2000, 3000–3009), it moves the household to the largest ID plus one (0x42b05e to 0x42b09f). The household keeps its file and record in memory, so its entries move with it (`OpenedPools::renumbered`, then `SaveInputs::renumber_family`). The pool family then made at the reserved ID has none.
4. Keep them to the families the opened neighborhood holds.

Checked against the original's run 2 of the UserData2 import (`SIMS_UD2_AFTER`):
- The open gives FAMh for families 1, 6, 11, 12 and 13, with payloads equal to the recorded save's.
  - Family 1 keeps the FAMh `Neighborhood.iff` held.
  - Families 6, 11, 12 and 13 take theirs from `Kat_6`, `Burb_11`, `Goth Sr_12` and `Charming_13`.
  - `Hick_10` and `Strays_4000` have none.
- The recorded save is imported into a store and given the open's FAMh. Saved through `save_neighborhood_iff`, it writes the recorded file byte for byte.

## Decision 3: creations nest at most 16 deep

`Interpreter` and `Machine` carry `creation_depth` (public, 0 for a run no creation started), copied whenever one makes the other.

The ordinary, multi-tile and person factories each check the depth before they register anything. The person factory checks it before it looks up the skeleton, since that lookup can load `.ndx` catalogs and draw from the C runtime stream. They then run the new object's Init and first turn (for a person, the construction reset's Init and update) on an interpreter one level deeper. Anything those scripts create therefore counts from there.

Create New Object Instance runs more of the new object's scripts after the factory returns: a visitor's "init visitor" tree (flag 0x4) and the placement's callbacks. That tail runs one level deeper too. Otherwise a visitor whose "init visitor" makes another visitor from its own record would nest at a constant depth until the stack overflows. The same holds for the temporary person a UI Effect or Dialog makes for an off-lot portrait: its put (with the search and the Placed callback) and its removal run one level deeper. Each creation still counts once.

Once 16 creations are nested (`MAX_CREATION_DEPTH`), the next one faults:
- The fault is on object 0, an unsupported trap naming the type and the limit.
- Nothing of the refused object is registered, and no skeleton or appearance is loaded for it.
- The fault reaches the script that asked through its primitive's usual fault path (Create New Object Instance, a UI Effect's or Dialog's portrait, Generic Sims Call 19). In strict mode that is an error; in play mode the faulting object is reset.
- A pet refused because neither pet expansion is installed is still refused, not faulted, as the original refuses it. That includes a pet named only by its skeleton (an empty body row 0). The skeleton lookup then runs on a copy, and is kept only for such a pet, since the original refuses it after the lookup.

The original's own behaviour for nested creation is not known, and the limit is not modeled. Like `TURN_BUDGET`, the bound is a guard. 16 is chosen for two reasons:
- No test or pinned run comes near it: the whole `sims-vm` suite and the Goth `TICK_43200` pin pass unchanged. Shipped content's deepest chain has not been measured.
- It keeps the synthetic chains within a default 2 MiB test thread in a debug build. Measured with the synthetic self-creating type, the 16-deep chain needs between 0.5 and 0.64 MiB of debug stack, about 32–40 KiB a level. That holds only for the tested configuration: a real object's deeper trees take more stack a level.

The Build and Buy cursor factories are entered from player commands, at depth 0, so they keep their own entry. Anything their Init creates is counted by the ordinary factory.

The synthetic tests show seven things:
- A type whose Init makes its own kind registers exactly 16 objects and faults, on the test thread's default stack.
- A type that does so in its first turn does the same.
- A visitor whose "init visitor" makes another visitor from its own record does the same. Without the tail's extra level it overflows the stack.
- A chain that stops one level short of the limit finishes.
- A UI Effect's or Dialog's portrait, one level short of the limit, still makes its temporary person, but an object made by the person's Placed callback is refused. Without the extra level it is made.
- A person refused at the limit is refused before its skeleton is looked up, with the lot and the C runtime stream unchanged.
- A pet named only by its skeleton is refused at the limit as below it: Create New Object Instance takes its false branch. Without the copy it faults.

The Goth `TICK_43200` pin is unchanged.

## Decision 4: small cleanups

- `legacy/fixtures/suites/` is removed: nothing reads it since `sims-runtime` went (#449). It stays readable at `legacy-final:fixtures/suites/`.
  - `.github/repository-policy.json` still lists the manifests' fingerprints under `reviewed_data`. The hygiene check does not require a listed path to exist, and `--history` still meets those blobs in past commits, so the entries stay.
- `docs/repository-workflow.md`:
  - The avatar-codec selectors now name the module in the one `sims-formats` integration binary (`--test integration avatar_codecs::…`). Each selector was run: the group runs 16 tests, the `--exact` example runs 1, and the old `--test avatar_codecs` target no longer exists.
  - The CI paragraph now describes what `ci.yml` runs: fmt, clippy and workspace tests on Ubuntu and macOS. It replaces the separately listed suites that CI no longer has.
- `crates/sims-sim/src/pose.rs` names `crates/sims-vm/tests/integration/resize.rs`.

## Consequences

- An imported family's FAMh survives into the saved `Neighborhood.iff` from both import paths. The `.FAM` path writes it through the store. The `Patch` path hands it to whoever saves the opened neighborhood.
- A self-replicating object faults at once, independent of the stack size. The `sims-antics test` runner's 2 GiB thread is no longer needed for that case, but it does no harm.
- Not done:
  - `export_family_fam` still writes no FAMh, though the original's exports carry one (the UserData2 `Patch` files do). Writing it needs the store's save inputs at export time.
  - `docs/repository-workflow.md` still cites the legacy `--test inventory_operations` selector of `legacy/crates/sims-vm`, which is not built.
  - `docs/legacy-mining-audit.md` still cites `tests/resize.rs`.
