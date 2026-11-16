# The game's speed and pause are the simulation's globals

*Decided 2026-11-15.*

The window paced the lot by itself: a tick every 50 ms at normal speed,
a third and a tenth of that faster, and no ticks while paused. The
simulation's globals were not touched. The original's code says
otherwise, so the window now does what its buttons do. Read from the
disassembly; none of it is checked against a run.

- The speed buttons call the setter (`0x46d697`) with 760 (normal,
  `0x4425a2`), 924 (fast, `0x46d264`) or 994 (ultra, `0x46d26a`). The
  setter clamps the speed to 0 to 1000 and keeps it in global 16. While
  global 18 is not negative, it keeps it there too. The time between
  two lot updates is 172 − speed × 0.001 × 171 ms, plus 0.5 and
  truncated (constants at `0x666a24`, `0x667720`, `0x66771c` and
  `0x667270`). That is 42, 14 and 2 ms. `sims_sim::speed` ports this,
  and `Lot::set_speed` takes it between ticks.
- The idle loop (`0x46d9d6`) runs an update each time a period has
  passed. The update (`0x46d345`) never reads the speed. The speed
  reaches the simulation only as globals 16 and 18. What reads them
  there was not looked for.
- Pausing (`0x46d813`) sets global 17, and running again (`0x46d83a`)
  clears it. The idle loop does not look at it, so a paused game goes
  on updating the lot. Each pass is then a partial one (`0x46d553`,
  already ported): only objects with scheduler flag 2 run,
  and the clock stands. `Lot::pause` and `Lot::unpause` do this. The
  window goes on stepping a paused lot. Strict mode on the Goths' house
  runs 200 paused ticks, and its clock stands through them.
- In live mode the speed buttons (`0x53a62a`, `0x53a6ad`, `0x53a728`)
  also run a paused game again. They ask the interface for it
  (`0x4cee4a`), which applies the speed first and then the unpause
  (`0x4ce5cc`, `0x46d83a`). `Game::set_speed` does the two the other
  way round. Both clear the same clocks with no tick between, so the
  order makes no difference. A button for the speed the game already
  has does not call the setter (`0x4ce588`), so the time owed carries
  on. Sol's review found both.
- **Unverified:**
  - The pause is refused while a field of the interface
    (`[0x6f2a58]+0x7c`) is not 0. Here it is never refused.
  - The setter and running again restart the idle loop's clocks
    (`+0xb8`, `+0xbc`), after which an update may be due at once. Here
    the time owed starts again from 0.
  - The original also caps how far it catches up (`0x4732de`). We keep
    our cap of 20 ticks a call.
  - Message 0xc2, which the setter and the pause send to the interface,
    and its effect on the sounds, are not ported.
- **Ported: the idle loop's own speed** (`World::idle_speed`, run by
  `Lot::step` before every update, in strict and play mode).
  - On a residential lot (`0x40745f`: zoning 1, `0x42b9e3`), before
    its updates, the idle loop asks whether anyone holds the game to the
    player's speed (`0x442f9b`, `World::speed_held`). A person in the
    registry's list holds it when all of these are true:
    - object data word 34 (`+0x8e`) is 0;
    - person data word 65 (`+0x60e`) is at most 1;
    - motive 11 (`+0x7b8`; motives start at `+0x78c`) is not below 0.0
      (`0x45edc5`, against the 0.0 at `0x66657c`; a NaN counts as
      below, as the x87 compare branches);
    - person data word 32 (`+0x5cc`) is 0.
  - While nobody holds it and global 18 is negative, the idle loop goes
    to 1000 (`0x46d790`): the setter makes the speed 1000, then global
    18 keeps the speed it was. While global 18 is not negative,
    `0x46d790` does nothing (`0x46d795`). So a speed the player sets
    meanwhile (the setter writes global 18 too) stands, not 1000.
  - While someone holds it and global 18 is not negative, the idle loop
    goes back (`0x46d7db`): the setter makes the speed global 18's,
    then global 18 becomes -1.
  - Both also restart the idle loop's clocks and send message 0x109 to
    the interface (1 going to 1000, 0 going back). That is not ported.
  - **Unverified:**
    - What word 34, word 65 and motive 11 mean. They presumably mark a
      Sim who is hidden (away at work or school) and one who is asleep.
      The full Goth day will exercise this when the household leaves or
      goes to bed.
    - Whether the original's speed changes on the Goths' house in the
      ticks we compare. The strict boot (House05, 1102 ticks) is
      unchanged by the port, so through it someone always holds the
      speed and global 18 stays -1.
  - `Lot::step` runs it before every update. The original's idle loop
    runs it on every pass, whether an update is due or not and paused
    or not, so `Lot::idle_speed` runs it between ticks too, and the
    window calls that on every pass of its own. A speed the player
    picks after the switch to 1000 then stands, as in the original. The
    check is idempotent: a second call in the same state does nothing.
  - The window follows the simulation's speed: ticks are owed at the
    period of global 16 (1 ms at 1000). Whenever the check calls the
    setter, even for the speed the game already has (going back to it),
    the time owed starts again and that pass runs no more ticks, as the
    setter restarts the clocks. The original may run an update at once
    then, if its second clock (`0x4732de`) is ready (`0x46da1b`); here
    the next tick waits a period of the new speed (**unverified**,
    window timing only). The panel's buttons still show the player's
    choice.
  - Sol's reviews found the guard that stood here before was wrong both
    ways, and that the check has to run between ticks as well. It failed where `0x46d790` keeps global 18's speed. It missed
    hidden and sleeping Sims, whose words it took to be unkept. Porting
    the whole head of the idle loop replaced it.
- Strict `sims boot` never calls the speed or pause hooks. The idle
  loop's speed never changes on the Goths through tick 1102, so the
  output is unchanged.
- **With the dialogs (#171).** While a dialog waits for the player, the
  window owes no time and does not run the idle loop's check: the lot
  takes nothing between ticks then (`Lot::idle_speed` is refused, as
  orders are). Once the dialog is answered, the time owed starts again.
  Whether the original's idle loop sets its speed while a dialog is up
  is **unverified**.
