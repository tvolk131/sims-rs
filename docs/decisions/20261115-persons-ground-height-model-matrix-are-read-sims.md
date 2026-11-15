# A person's ground height and model matrix are read from sims-sim

*Decided 2026-11-15.*

`sims_sim::head::ground_height` and `sims_sim::head::model` give the
terrain's height under a person and their model matrix (0x592132) from a
`World` and an animator position. They only read the world and add no
state, so the world's hash is unchanged. They follow `sims_vm`'s head
model line for line: the same rounding (a half added, truncated in
single precision, NaN to 0 as `__ftol` gives it), the same tile cut to a
signed byte, and the same order of offsets, which the head captures pin.
They also refuse a tile beyond the 64-tile arrays when a lot's size
says more. sims-people now uses them instead of its own copy. `sims_vm`
keeps its private copy for now; it could call these instead. An upper
level still fails (`Unplaced::Level`): the view-table offset is not
ported.
