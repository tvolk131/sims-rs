# The game paces the lot itself, and runs it strictly

*Decided 2026-11-15.*

A tick runs every 50 ms at normal speed, the animator clock's step.
Fast and ultra run three and ten times as often (**unverified**: the
original's are not measured). At most 20 ticks run for one wake of the
window; time beyond that is dropped, so a stall never makes the lot
race. Pause stops stepping the lot; the panel's pause button shows it.
The speed and pause stay outside the simulation until it takes player
orders. The lot runs in strict mode with the captured frame schedule
(`Frames::default()`), so it plays exactly the states the captures
check; the first failure stops it and is shown on screen. Clicks pick
objects by their tile until `sims-render` can say which object drew a
pixel. Reopen when the play-mode step and player orders land.
