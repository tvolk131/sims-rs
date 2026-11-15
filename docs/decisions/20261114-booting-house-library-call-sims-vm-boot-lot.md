# Booting a house is a library call: `sims_vm::boot::Lot`

*Decided 2026-11-14.*

The drawing tracks asked for a way to run a lot one tick at a time and
read its state in between. The load sequence and the tick loop were in
`sims boot`. They now live in `sims_vm::boot`:

- `Lot::load` runs the load and reports each step.
- `Lot::step` runs one lot update, then the house view's frame when the
  schedule (`Frames`) draws one.
- `Lot::world` reads the state.

A step that fails in its tick or a frame leaves the lot partway through
that work, so the lot halts: every later step fails and changes nothing.
The old CLI stopped there too. The motive constants are worked out at
the first step rather than at the load, so a load that runs no tick
doesn't need them, as before.

`sims boot` prints what the library does, and its output is unchanged
byte for byte: at K = 366 it gives the same hash, and at tick 367 the
same stop.

Why `sims-vm` and not a new crate: the boot needs the interpreter, the
linked program and the content loaders, which `sims-vm` already depends
on. A crate that depends on `sims-vm` reads the world through `&World`
only, so presentation never writes simulation state.

A frame that fails after its tick still reports that tick's update, as
the CLI listed it before. Commands from players will enter through
`Lot` when the command intake exists.
