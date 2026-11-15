# The game's sound inputs are guesses until measured

*Decided 2026-11-15.*

`sims-game` feeds `sims-audio`'s player two inputs the simulation does
not own. The sound scripts' speed register (global register 100,
`SimSpeed`) is 1 at normal speed, as `sims-audio`'s `play` example
uses, 2 at fast and 3 at ultra; the ambience's view, which the original
works out from the camera (0x570c3e, not read), is held at 50, the
value that plays the backyard ambience in that example. Both are
**unverified**. The ambience's hour is the clock's (global 0). The
mixer runs on its own thread, a fifth of a second ahead of the device;
when it falls behind, the device plays silence rather than the game
waiting. Reopen when the original's view value or speed register is
read.
