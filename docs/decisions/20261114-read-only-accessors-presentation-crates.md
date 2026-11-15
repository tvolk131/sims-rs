# Read-only accessors for the presentation crates

*Decided 2026-11-14.*

The render track asked for two ways to read the simulation without touching it:

- `ReadOnlySnapshots` opens an existing snapshot file with redb's read-only
  open: it never creates the file, never repairs it and never commits, so a
  viewer can read a snapshot another program keeps (several readers share
  it; a writer holding it locked refuses the reader). `Snapshots::open`
  still creates the file, for the CLI that writes it. Both share one load,
  with the same format check.
- `World::selected` and `World::selected_whereabouts`: the selected Sim's
  ID, position, tile, level and room. The room is the object's room word
  (data word 29), which the placement writes whenever the Sim is put
  somewhere, rather than a new lookup in the room map, so the presentation
  shows what the simulation holds. `sims_vm::selection::selected` now
  calls `World::selected`.

Neither changes the state or its encoding. Reopen if a presentation crate
needs something the state does not hold.
