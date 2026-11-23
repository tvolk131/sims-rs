# Legacy crates are deleted one at a time, once fully mined

- **Date:** 2026-11-23
- **Status:** Accepted
- **Scope:** `legacy/crates/`, `legacy/README.md`, `docs/salvage-audit.md`, `docs/legacy-mining-audit.md`

## Context

Until now, everything under `legacy/` was read-only and kept whole. The salvage audit
(`docs/salvage-audit.md`) graded each legacy crate PORT / REFERENCE / DROP, but
nothing was ever removed. As a result, 670k lines of unbuilt code sat beside the
new workspace. Readers could not tell which parts still held something to port and
which were dead.

The user changed the rule on 2026-11-23:

- a whole legacy crate may be deleted once a mining audit proves nothing in it is
  left to port;
- editing legacy code is still not allowed;
- everything stays recoverable from the tag `legacy-final` (e1b72647, on origin).

## Decision

1. **The bar for deleting a crate is "nothing left to mine".** Check every
   module's knowledge against the current crates:
   - primitive semantics;
   - native addresses and constants;
   - file-format details;
   - routing, render and sim rules;
   - test fixtures and expected values.

   Then classify each item:
   - **ported**, citing its new location;
   - **deliberately dropped**, citing the vision, an ADR or the salvage audit;
   - **still to mine**.

   One still-to-mine item keeps the whole crate. If in doubt, it stays.
2. **This round removes `sims-kernel`, `sims-tools` and `sims-fixtures`.** None of
   them holds game knowledge:
   - `sims-kernel` is the Win32 named-mutex and event emulation.
   - `sims-tools` is the old inspector CLI. `sims-cli` and `sims-antics lift`
     replace it.
   - `sims-fixtures` is the private evidence store.

   No file outside `legacy/` cited any of them.
3. **Seven crates stay, each with a written backlog:**
   - `sims-app`
   - `sims-content`
   - `sims-formats` (leftovers only)
   - `sims-render`
   - `sims-runtime`
   - `sims-sim`
   - `sims-vm`

   `docs/legacy-mining-audit.md` holds the per-crate audit tables and the
   consolidated porting backlog. A crate becomes deletable when every backlog
   item for it is ported, or explicitly dropped with a cited reason.
4. **Cite a deleted crate through the tag.** Outside `legacy/`, write
   `legacy-final:legacy/crates/<crate>/<path>`; it resolves with
   `git show legacy-final:legacy/crates/<crate>/<path>`. When a crate is deleted,
   rewrite every `legacy/crates/<crate>/` citation in the same change. This
   round needed no rewrites.
5. **Delete one crate or group per PR, each opened after the previous one merges.**
   Each PR updates:
   - `legacy/README.md`;
   - `docs/salvage-audit.md`;
   - `docs/legacy-mining-audit.md`;
   - any path in `.gitignore`, `.github/repository-policy.json` or
     `scripts/check_repository.py` that names the crate.

## Consequences

- About 13k lines leave the tree. The remaining legacy crates are the ones that
  still hold work.
- The backlog in `docs/legacy-mining-audit.md` turns "read legacy for behavior"
  into concrete items. The biggest are:
  - the exploration (determinism and snapshot) fuzzer, from `sims-runtime`;
  - footstep sound selection and the unported animator states, from `sims-sim`;
  - the censor mosaic, from `sims-render`;
  - the community-lot RNG reseed and the date and indicator queues, from `sims-vm`;
  - buy-catalog eligibility and naming, and object graphic selection, from
    `sims-content`;
  - non-modal and tutorial dialogs and the tutorial highlight, from `sims-app`.
- The remaining legacy crates' `Cargo.toml` files still name the removed crates.
  Nothing in `legacy/` is built, so this is harmless. Editing those files would
  break the no-edit rule.
- `legacy/fixtures/suites/` stays. It is consumed by the legacy `sims-runtime`
  conformance runner and is removed together with that crate.
