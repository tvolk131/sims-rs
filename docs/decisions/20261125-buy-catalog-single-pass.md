# The Buy catalog reads each object file once

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-content/src/buy_catalog.rs` (`Catalog::buy_entries_visiting`, `BuyVisit`), `crates/sims-ui/src/buy.rs` (`BuyCatalog::load`), `crates/sims-ui/tests/integration/buy_load.rs`
- **Track:** Track 4

## Context

Since #452, `sims-ui`'s `BuyCatalog::load` takes its listing and catalog
text from `Catalog::buy_entries`. It still reads the words only the
interface needs from each definition's `OBJD`, `TTAB` and text chunks:

- the sort words, price and ratings;
- the usage restriction;
- the fallback names of definitions it does not list;
- each part's master.

`buy_entries` read and parsed every object file, and then `load` read and
parsed every object file again for those words. #452 measured the load at
about 385 ms, up from 289 ms.

## Decision

`sims-content` gains `Catalog::buy_entries_visiting(index, language,
visit)`. It is `buy_entries` with a callback:

- **When it calls.** `visit` runs once for every definition the pass
  parses, listed or not, while that definition's file is open.
- **What it passes.** A `BuyVisit`: the object number, the open `Iff`,
  the `OBJD` chunk, the parsed `BuyDefinition`, and the `BuyEntry` if the
  catalog lists it.
- **`buy_entries` itself** is `buy_entries_visiting` with an empty
  callback, so its output and diagnostics are unchanged.
- **The rules stay in one place.** Eligibility, interaction tables and
  catalog text are decided only in `sims-content`.

`BuyCatalog::load` now reads everything from those visits:

- **Which definitions.** It keeps the visits of each GUID's registered
  definition (`Catalog::by_guid`), as its own pass did.
- **Unreadable files.** A file, `OBJD` chunk or definition that cannot be
  read is visited by neither pass, so it is left out exactly as before.
- **Masters.** The old pass read files GUID by GUID, so where one file
  had two masters with the same ID, the greater GUID won. The single pass
  goes in registration order, so it keeps the greater GUID explicitly.
  Each part is resolved against its own file's masters after the pass.
- **Order does not matter.** Items are sorted by price, raw name and
  GUID (a total order), and the other two maps are keyed by GUID.

The other callers of `buy_entries` (the catalog tests) are unchanged.

## Verification

- **Dump.** `crates/sims-ui/tests/integration/buy_load.rs` writes the
  whole loaded catalog (`{:#?}`) to `SIMS_BUY_DUMP`. The dump over the
  complete collection is byte for byte the same before and after. It
  holds game text and is never committed.
- **Timing.** The same test times five warm loads. See the PR for
  before and after.
- **UI scores.** `ui_oracle_tour` (`buy-top-function` 1.0000,
  `buy-subcat-seating` 0.9979) and the 8/8 ratchet are unchanged.

## Consequences

- `sims-content` gains a public type and method, `BuyVisit` and
  `Catalog::buy_entries_visiting`. No existing signature changes.
- A caller that needs more of each object file than the catalog's rules
  can read it during the same pass, rather than parsing the files again.
