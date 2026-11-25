# Shell Travel from the VM Install Cache; Buy Catalog from `sims_content`; Options in `<saves>.userdata`

*Decided 2026-11-25.*

**Branch:** `t3/game-cached-travel-buy-catalog-and-cleanups`
**Track:** Track 1

## Context

- **Travel.** `NeighborhoodShell` called the uncached `sims_vm::travel` functions, so every departure, return, resume, abort and destination edit rebuilt the install-wide VM state that `enter_lot` already reused. Go Somewhere Else (tree 0x1016) on a destination was declined with "not supported yet", although `travel_on_from_request` exists.
- **Buy catalog.** `sims-ui`'s `BuyCatalog::load` kept its own copy of the native Buy eligibility rule and the name and description lookup (CATS, then CTSS). `sims_content::Catalog::buy_entries` (#441) now owns both.
- **Options file.** The shell and the standalone binary saved Options as `saves.with_extension("options.iff")`. Two saves files differing only in extension (`saves.redb`, `saves.backup`) shared one Options file. #442 already keeps `Neighborhood.iff` in a `<saves file name>.userdata` folder.
- **Duplication.** `Game::open_from_store_cached` and `Game::from_loaded_lot_cached` each spawned the same three asset workers. The two house openings each built an index and catalog the same way. `Mixer::open` and `Mixer::open_pack` each built the same four streamed players.

## Decision

### Travel

- **Every shell travel call uses the warm cache.** The calls are open-for-edit, depart, depart-from-request, travel-on, return-from-request, return, resume and abort. Each passes `InstallAssets::vm_cache()` to the `*_with_cache` variant, and the new lot is wrapped with `Game::from_loaded_lot_cached`.
- **Travel needs the install assets.** If they fail to load, travel stops the load audio and fails with the error, the same as a VM error. `enter_lot` already did this. Aborting an interrupted trip is the exception: it opens no lot, so it falls back to the uncached VM call and the trip can still be recovered.
- **Go Somewhere Else travels on.** `pump_trip_request` sends a request on a destination lot, for a house other than home, to `travel_on_from_request_with_cache`, as `Departure::Onward`.
- **What it still refuses.** Co-op, an edited destination, a house that is not a travel destination, and any VM error each get a notice and a declined request. The VM error includes a party that would leave a traveler behind, such as a date whose companion has not arrived.
- **Departure checks.** A plain departure while a trip is active, and a trip on with no active trip, are refused before the VM is called.

### Buy catalog

- `BuyCatalog::load` lists objects from `Catalog::buy_entries(index, US_ENGLISH)`, taking each object's eligibility and text from it.
- `sims-ui` keeps only its presentation work:
  - grouping master and part objects;
  - the `any_sort` and listing filters;
  - the TTAB usage flags;
  - icons.
- **Deleted:** its eligibility function, CTSS reader, OBJD field offsets and the matching unit test.
- **Names change for some objects.** The native rule names an object with an empty catalog name `*` plus its OBJD label, and reads CATS as description first, then name. Some names therefore change. They now match the original.
- **Unlisted objects** use the same `CatalogText::read` rule for their fallback names.
- **Unreadable catalog text.** `buy_entries` leaves out a definition whose CATS or CTSS cannot be read. `BuyCatalog::load` still lists such an object when it has a catalog sort word, so a damaged string table cannot hide it. It is listed as `*` plus its OBJD label, with no description. On main it was listed with an empty name. The earlier listing rule still applies: the object needs graphics, a master, or an interaction table, so damaged text never puts an object on sale that would not be listed anyway. An unlisted object with unreadable text is named the same way where the game shows its name, for example when selling it. No object on the reference install takes this path.
- **`BuyLabels` is not used.** Neither crate had its own category-label loading to replace.
- `sims-game`'s three Buy catalog loads share `hud::load_buy_catalog`.

### Options file

- **New location.** Options are saved as `<saves file name>.userdata/Options.iff`, beside #442's `Neighborhood.iff` folders. The helpers are in `sims_game::user_files`.
- **Who uses them.** The shell, the opening path and the standalone binary all use the same helpers.
- **Backward read.** When the folder has no `Options.iff`, Options load from an older build's `<saves stem>.options.iff` if present. The next save writes the folder. Nothing writes the sibling file any more. The old file names no saves file, since every saves file with that stem wrote and read it. So any of them may load it until it saves Options of its own, as before.
- **The CAF names sidecar stays a legacy sibling** (`<saves stem>.caf-names.tsv`). Nothing writes it. It is read once, to migrate into the store. Its rows are keyed by neighborhood and instance ID, so a shared file cannot migrate into the wrong store.

### Deduplication

- `game/opening.rs`:
  - `load_lot_assets`, `load_hud_static` and `load_people_assets` replace the two copies of the asset workers;
  - `catalog_with_characters` replaces the two house-opening character and catalog builds. Each caller still builds the index where it did before, so the same error wins when several fail.
- `sound.rs`: `streamed_players` replaces the two copies of the mode music, radio, ambience and voice setup.
- No behaviour change is intended.

## Verification

- **Integration tests** in `trip_requests_and_neighborhood_iff`:
  - A solo trip to lot 21 travels on to lot 22. The trip ID is new and the home is kept. `Neighborhood.iff` records the move, the traveler arrives, and the trip returns home.
  - A Go Somewhere Else while a date's companion has not arrived is declined with a notice. The trip and lot are unchanged.
  - Options are saved in the folder and not as a sibling. Another saves file does not see them. They reload. An older build's sibling file loads.
- **Unit tests** in `user_files` cover the extension collision and the sibling fallback.
- **Buy catalog:**
  - `ui_oracle_tour` scores are unchanged (`buy-top-function` 1.0000, `buy-subcat-seating` 0.9979);
  - the ratchet is 8/8 with unchanged shares.
- **Cost:** `BuyCatalog::load` takes about 385 ms, up from 289 ms, because each object file is read by both `buy_entries` and the icon pass.

## Follow-ups

- **Go Home on a trip-on destination.** The cab-sign Go Home after a trip on (to lot 22) produced no trip request within 3,000 frames in testing. The test returns through `NeighborhoodShell::return_from_destination` instead. This is VM or script behaviour (Track 2).
- **Read each object file once.** `buy_entries` could hand back the parsed file, or `sims-ui` could take its icons from the same pass.
