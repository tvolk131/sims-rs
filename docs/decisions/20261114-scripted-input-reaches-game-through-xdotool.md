# Scripted input reaches the game through xdotool, and is confirmed in memory

*Decided 2026-11-14.*

The harness can now drive the original from a scenario file while it
captures (`capture.py --scenario`, `tools/harness/input.py`). This is how
captures with player input are made, for our simulation to replay.

What was decided:

- **Input goes through xdotool in the guest, not QEMU.** QEMU's input
  devices do not reach the game: the guest's X server is Xvfb, which reads
  no device. A small guest job (`guest/input-agent.sh`) runs the commands
  the host drops in the share, polling every 50 ms; the generic job runner
  polls once a second. Guest memory is never written.
- **What was clicked is read from the pie menu, not trusted.** The
  picker tool's hovered object (picker `+0x2e`) lags the pointer by tens of
  lot updates under TCG, because the game takes pointer moves late. Once it
  named the bench while the pointer was over the porch floor, and the click
  opened "Go Here". So the hover reading only says when to press, and the
  pie menu's actions say what was clicked: a menu whose actions aim at
  another object is closed with Escape and the next aim point tried.
- **A chosen interaction is confirmed twice.** capture.py stops at
  `0x48ddcb`, the call in the pie menu's select handler (`0x48dd67`) that
  pushes the action on the person (`0x45d20b`, edi the person, esi the
  action), and logs it with the update it falls before or in. Then
  `expect-queued` waits for the person to hold that action, queued
  (`+0x820`) or running (`+0xa48`).
- **Dialogs are answered on the dialog stops.** A modal dialog holds the
  lot update it shows in, so a step that waits for update returns would
  never see it. `answer-dialog` presses once, as `answer.sh` does, and
  waits for the dialog's result stop.

**Checked against the original.** In a capture from a fresh boot
(2026-11-14), the scenario answered the five welcome notices and the Help
System dialog, aimed at bench 173 by projecting its position (hover
confirmed on the second aim point), opened a pie menu with the one entry
Sit (caller 21, target 173, routine 8213, priority 50), and chose it. The
push came before lot update 178. At the return of update 180 the person's
queue was already empty, and the full state after update 200 holds Sit on
173 as its active action, with uid 1. (That run's check read only the
queue; reading the running action too was added after it.) A second
scenario on the same running lot hit the bench at the same aim point, got
the same menu, recorded the push before update 168, and found Sit on 173
in person 21's queue (uid 7) at the return of update 168, behind its
running action. Screenshots showed the bench in the
person's queue. The memory layout used (picker, pie menu, people panel,
house view, queue) is listed in `input.py`; each part was read in the
running game.

**Unverified.**

- The projection was checked only in the starting view (zoom 1, no
  rotation), and it ignores the floor's height: the porch, on a
  foundation, is hit about 30 pixels above its projected point, which the
  aim points cover.
- Hovering is not free of side effects: `0x48c834` sets and clears
  highlight bits in the hovered object's flags (`+0x114`, through
  `0x4320da`). Whether those bits reach anything our simulation compares
  is not checked. A replay may need them, or a check that they are only
  drawn.
- Submenus, build and buy mode, and a missed dialog press (the step then
  waits for the dialog to be answered by hand) are not handled.
- Dialog button places are written in the scenario, not read from the
  dialog.
