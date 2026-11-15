# The interface's tests and the viewer's panel open houses through `sims_vm::boot::Lot`

*Decided 2026-11-15.*

The `sims-ui` tests (`tests/common`) and `sims-viewer --hud` now load the
house as the game loads it, through `sims_vm::boot::Lot::load`, up to the
frame before the first tick. Until now both read the house file into a
fresh world, and the tests stood in for two of the load's steps by hand:

- step 17, the lot's funds set to the family's;
- step 18, the family's first member selected.

`Lot::load` runs the whole household load, so both steps happen as the
game does them, and so do the neighborhood's people, which the
relationship panel reads.

- **What changed on screen:** the funds and the selected Sim are the same
  as the stand-in gave ($2,542 and Mortimer, as the original showed). The
  motives are not: the house file holds fractions (two of Mortimer's
  are 99.625 and 71.45), and the load leaves whole numbers (100 and
  95).
  So the needs panel's two open-strip digests in `tests/hud.rs` are
  pinned again. The other digests are unchanged.
- **The viewer** boots the lot only with `--hud` and no `--snapshot`;
  without the panel it still shows the house file's world, which needs
  no load. The world is cloned out of the lot; the viewer never runs it.
- **The tests** still build their own index for the art, the faces and
  the careers, since the lot does not hand its index out. The catalog is
  the lot's.

What would reopen it: a screenshot of the original's needs panel right
after loading that shows the house file's fractions rather than the
load's values.
