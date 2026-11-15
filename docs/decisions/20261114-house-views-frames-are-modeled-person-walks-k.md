# The house view's frames are modeled, a person walks, and K = 230

*Decided 2026-11-14.*

Our state now equals the 1000-tick capture of the Goth house after each
of ticks 1 to 100 and after every tenth tick from 110 to 230, in
everything compared, but for global 12 after tick 1 (already known). The
capture has no full state after ticks 231 and 232, and tick 233 stops at
Find Best Interaction (primitive 3), which is not ported. K goes from 33
to 230.

Some terms. A *frame* is one drawing of the game's window; the *house
view* is the part that draws the lot. A *write watchpoint* is a hardware
breakpoint that stops the game when it writes given bytes; it gives the
instruction that wrote (its address, EIP) and, through the saved frame
pointers, the return addresses of the calls that led there (the *call
chain*). The *animator* and its *state* are as in the entry below.

What was decided:

- **The first frame that draws a person writes its animator, and that
  is modeled** (`crates/sims-vm/src/frame.rs`,
  `Interpreter::draw_frame`). After tick 34 the capture had person
  259's direction and destination direction at 0. A run of the original
  with a write watchpoint on the direction (`capture.py --watch`)
  stopped at 0x584895, outside the lot update, between updates 33 and
  34. Call chain: 0x45e36b (the person's draw), 0x4ab67d, 0x4a945d,
  0x4a8deb (the house view's draw), 0x4d1bbb, 0x4d0dea
  (`DDDSimsView::Draw()`), 0x5dcffe (the main loop). It is the
  animator's draw (0x590c05). Its byte +0xdb is set when the animator
  is made and cleared by the first draw, which also unsets the position
  (0x589361) and places the animator on its person's tile, facing the
  person's direction word (0x592132). Drawing is presentation, but this
  writes state the next tick reads, so it is modeled. The rest of the
  draw (the pose, bounds, screen rectangle, the skin's censor update) is
  not; that it writes nothing the simulation reads is **unverified**.
  Where the draw would need more than this (a skin waiting for its
  update, a head turn, a body that follows person word 8, a person in
  the house view's second list), it fails loudly. Byte +0xdb is kept as
  `Animator::undrawn`; snapshot format 14.
- **When frames are drawn is an input.** A second run watched the house
  view's draw guard (written at 0x4d1b60 and 0x4d1be7). The view drew
  before the first update, after updates 1, 2 and 3, and after update
  33, and not after updates 4 to 32 or 34 to 45. People 16, 18 and 21
  were drawn by the first frame (their +0xd8 to +0xdb were already 0
  before update 1). That timing comes from the main loop and the window
  system, not the simulation, so `sims boot --frames` takes it, and the
  default is `0,1,2,3,33+` (0: before tick 1; 33+: after every tick from
  33 on). Checked: with that default, and with `0,33+` or
  `0,1,2,3,33`, the state after tick 232 is the same and every compared
  tick is equal; with only `0`, 82 lines differ from tick 34 on. Why the
  original drew nothing after updates 4 to 32 is **unverified**; it
  depends on how the harness paces the game.
- **Registry byte 0x12 is set at the end of the object read**
  (`World::finish_object_read`). A watchpoint on it stopped at 0x43fc8d
  (in 0x43fc02, after load steps 8 and 9), going from 0 to 1. It stopped
  again at 0x442521, in update 36, going to 0: the end of a full lot
  update clears it when the tick count is a multiple of 40 (0x4424cb),
  now ported too. Both are simulation. The load's state, and
  the state after tick 1, now match the capture on this byte; before,
  ours set it in tick 2.
- **The walk is ported** for what the house reaches (0x58eb29 to
  0x58f089, `crates/sims-vm/src/animator.rs`). Facing the point ahead,
  the animator starts the walk clip repeating (layer end mode 1,
  0x597dca), keeps the old clip's phase if that clip moves the person
  (clip +0x34, `Clip::moving`), moves each tick by the clip's distance
  times 16/3 over its duration at its speed in ticks of 42 milliseconds,
  moves the step onto the path where that is nearer (0x48313e,
  0x45ed5f), puts the person on the step's tile with the place test,
  and notes its room (0x45e43d, moved out of the person's tick as
  `note_room`). Byte +0x19c, whether it walks, is kept
  (`Animator::walking`). A walk that would stop short (0x58ed52) or is
  blocked (0x58f682) fails loudly.
- **Timed events are ported** (0x5912fd, `clip_events.rs`): `xevt`,
  `interruptible`, `lefthand`, `righthand` and `censor`. Sounds are not
  modeled. `anchor` is not kept: it feeds only code behind byte
  0x6f6d70, which no code writes and which is 0 in a memory capture of
  the running game. That nothing else reads a layer's anchor fields is
  **unverified**. `dress` and `undress` fail. Animate Sim's clip 0
  stops the foreground clip (0x5897aa).

How it was checked. The capture's full state after each of ticks 1 to
100 and every tenth tick to 230, and its `updates.jsonl` line after
every tick to 232, were compared with ours field by field, floats bit
for bit, by a script outside the repository: the fields listed for tick
1 in `the_goth_house_first_tick_matches_the_original`
(`crates/sims-vm/tests/resize.rs`), the four people's animators with the
walking flag, and each person's number of route contexts. That test runs
to tick 232, pins the state, and checks that tick 233 fails. The pins
taken before this change still hold once the new fields and byte 0x12
are put back as they were (`format_13_bytes`, `with_read_flag_clear`).
The walk, the path snap, the repeat mode and clip 0 have synthetic tests.

What did not match: global 12 after tick 1, as before.

**Unverified:** the state after the ticks the capture skips (101 to 109
and so on); the draw's other writes; the frame schedule outside the
45 updates the second run watched.
