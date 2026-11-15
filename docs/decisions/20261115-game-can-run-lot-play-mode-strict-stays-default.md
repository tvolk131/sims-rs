# The game can run the lot in play mode; strict stays the default

*Decided 2026-11-15.*

`sims-game` steps the lot with `Lot::step` by default, which stops at
the first error, so what it plays is exactly what the captures check.
`--play` (`Options::play`) steps with `Lot::step_play` instead: an
object whose tick fails is reset (as the original resets an object after
a SimAntics error) and the lot goes on, so a human can play past the
first unported primitive. A lot played that way is not one to compare
with the original.

In play mode each new error caught is printed and written across the top
for 200 ticks (10 s at normal speed) with a running count. The same
error again at a later tick (or a frame skipped the same way again) is
only counted, so one that recurs every frame does not hold the line
open. The first 256 different errors are listed, later ones only
counted; after 4096 different errors every new one is only counted, as
it can no longer be told from its repeats. A failure play mode does not
catch still stops the lot. An error caught in a step that then fails is
not logged: `Lot::step_play` returns the failure without the step's
update. Within a step the log keeps the order things happened in: the
frame before the first tick, the objects' errors, the frame after the
tick (a single skipped frame on the first step is taken to be the one
before it, which the update does not say).
