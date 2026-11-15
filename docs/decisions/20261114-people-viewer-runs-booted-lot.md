# The people viewer runs a booted lot

*Decided 2026-11-14.*

`people-viewer --boot` boots the house with `sims_vm::boot::Lot` (the
library form of `sims boot`) and runs it in the window: one tick every
50 milliseconds, because the animator clock moves 50 a tick at normal
speed. It catches up by at most 10 ticks per drawn frame, so a slow
tick slows the lot down instead of stalling the window. After each tick
the people are drawn from the lot's state as they are from a snapshot.
People a script makes while the lot runs are read from their objects
when they first appear, each on its own, so one person that cannot be
read (a type missing from the install) does not hide the others. `--answers` answers dialogs as `sims boot` does
(the Goths' house takes `--answers 3`). `--png` with `--at` runs `--at`
/ 50 ticks and then draws one frame.

When a step fails, the run ends there. The viewer prints the failure and
keeps the last state on screen. Today the Goths' lot stops at tick 367,
on a route that is not ported. That is the end of what the simulation
runs, not a fault in the drawing.

The viewer holds the lot and reads it only through `Lot::world`. The
library does not depend on `sims-vm`: only the viewer does (behind the
`viewer` feature), and so do the tests.

Checked: a gated test (`tests/lot.rs`) runs the Goths' lot to its end
and draws everyone placed every 30 ticks. At most four people are
placed at once (the family and a visitor). Each draws on its own,
covering part of the frame. Looked at by eye, the people move between
ticks. Unverified: none of this is compared with the original. Ticks
run at 50 ms, so the lot plays at normal speed; the slowed clock (42 a
tick) is not used. The camera still centres on the average of the
people, so on a whole lot some of them are off screen until it is zoomed
out.
