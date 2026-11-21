# Special Effect message 0x10c resets the game speed to normal

*Decided 2026-11-21.*

## Context

`crates/sims-vm/src/primitives/special_effect.rs` previously left message `0x10c`
unmodeled as presentation, noting as **unverified** that the listeners on
`0x40760d` write nothing the simulation reads (see also
`20261115-multi-tile-factory-school-bus-comes-strict-mode.md`).

Comparing the full 44,200-update Goth day (`goth-day-20261115`) revealed a
divergence in `globals[16]` (game speed) from update 39,610 through 40,494:
at update 39,606 the raccoon (`obj 10`, GUID `0x5DAC929F`) spawns while
everyone on the lot is asleep at idle speed (`globals[16] = 1000`,
`globals[18] = 760`). At update 39,609 it runs Special Effect (`0x1007` node
19) state 0, and at update 39,610 state 1 with operand flag `0x40` clear;
native drops `globals[16]` from `1000` to `760` (`NORMAL`) at update 39,610
while keeping `globals[18] = 760` (so `World::idle_speed` at `0x46d9d6` leaves
the lot at `760` while everyone sleeps; when Mortimer wakes, `idle_speed`
restores `760` and clears `globals[18]` to `-1` at update 40,496 in the native
recording, and at update 40,495 under the port's fitted frame schedule).

## Evidence in the original binary

1. **Sender (`0x44abda`–`0x44abf4`).** In Special Effect state 1, once
   `shows_effect` passes (`0x44ab42`–`0x44abcc`), if bit `0x40` of operand
   byte 4 is clear (`not al; test al, 0x40; je 0x44abf5`), it calls
   `0x40760d(0x10c, 0)` to notify every listener registered on `[0x6de858]`.
   Scanning all 110 callers of `0x40760d` confirms `0x44abee` is the sole
   sender of `0x10c`.
2. **Listeners (`0x407577`).** All eight listener classes registered via
   `0x407577` (`0x41637f`, `0x43c0e4`, `0x44302e`, `0x46d2c2`, `0x47b22a`,
   `0x4a82db`, `0x4c3ea5`, and `0x4f65bf`) were inspected. Only the control
   panel's listener (`0x4f65bf`, vtable `0x669804`) handles `0x10c`
   (`0x4f65d7 jg 0x4f6731` → `0x4f6738 jg 0x4f6842` → `0x4f684c sub eax, 3;
   je 0x4f6856`), with no intermediate guards.
3. **Handler (`0x4f6856`–`0x4f687a`).** It calls `0x4425a2` (`mov eax, 0x2f8`
   = `760`, `sims_sim::speed::NORMAL`), reads `globals[16]` via `0x46d78b`,
   and if they differ calls `0x46d697(760)` (`World::set_speed(760)`), which
   writes `globals[16] = 760` and, while `globals[18] >= 0`, `globals[18] =
   760`. It then sets UI dirty bit `0x40` (`0x4ceceb`) and refreshes the panel
   (`0x4ce41b`, presentation only).

## Decision

In `Machine::special_effect` (`crates/sims-vm/src/primitives/special_effect.rs`),
when state 1 shows the effect and operand flag `0x40` is clear, check
`self.world.speed() != NORMAL` and call `self.world.set_speed(i32::from(NORMAL))`.
In `Game::run_while` (`crates/sims-game/src/game.rs`), when a tick changes the
simulation's speed or return speed, sync `pace.speed` from `pace::shown(world)`,
restart the owed time, and (in `advance`) end the batch so the panel and pacing
follow the reset. STR# `0x131` formatting (`0x44bf1a`) and effect rendering
(`0x4a6950`) remain unmodeled presentation.
