# The game plays the lot's sound requests as it wakes

*Decided 2026-11-15.*

`sims-game` takes each step's `Update::sounds` and hands them, in
order, to `sims-audio`'s player in the same wake of the window that ran
the step (it wakes every few milliseconds). What is heard comes later by
the output the player has already rendered, up to about 0.2 s. In strict
mode, a step whose frame fails after its tick still hands its sounds
over. `Play`
is `Play` and `Stop` is `StopObject`. The simulation's 16-bit object ID
is widened without its sign (**unverified**: what the original passes
for a negative ID). Until the host takes them, the game keeps at most
4096 requests, dropping the oldest, so a host that never plays sound
does not grow without end. `begin_route` and `end_route`, which Go To
Routing Slot requests, find no event in the install's sound files; the
player records a fault and plays nothing, which may be what the
original does too (**unverified**).
