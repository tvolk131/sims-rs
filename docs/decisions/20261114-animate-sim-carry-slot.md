# Animate Sim on the carry slot

*Decided 2026-11-14.*

Tick 417 of the Goth house stopped on an Animate Sim with flag 0x10
alone: person 21's bookcase tree (0x1014, node 19) plays a clip on the
carry slot (animator +0x13c). The original's path is `0x589683`.

- **The animator keeps the carry layer** (`Animator::carry`, snapshot
  format 21). Like the foreground layer, its update hands timed events
  to the animator, which scripts can read (the event queue, the hands).
  The base layer stays a flag because its clips have no events.
- **The path, as the disassembly shows it.** Clip 0 clears the slot
  (`0x592878`) and sets the carry mode to 0. Any other clip is looked up
  as for the foreground (`0x5893b3`, which reads the appearance again
  when stale). Flag 0x80 slows the clock. The clip starts in the slot
  (`0x592726`), backward with flag 0x02. The carry mode becomes 6, and
  the layer is set to repeat (mode 1) at priority 10 (`0x59867f`). Both
  ways the carry selection becomes -1 and the true branch is taken, with
  no yield and no events followed.
- **The skin's list order is kept** (`Animator::carry_first`). The
  skin's update goes through its layers in list order (`0x5992b3`), so
  whichever layer comes first has its timed events handled first.
  Every layer is made at priority 1 and put in before the first layer
  whose priority is above 1 (`0x599128`); its own priority is set
  afterwards. So a carry layer goes before a foreground layer of raised
  priority (6) and after one of priority 1. A new foreground layer
  always goes before the carry layer (10).
- **Removal is immediate.** `0x592878` fades a layer out only when byte
  0x6f6bd0 is set, which holds 0 in a running game. So the reset, a
  pickup clip (flags 0x11) and clip 0 each remove the carry layer at
  once.
- **Still failing loudly:**
  - a clip the lookup does not find (the original reports it and goes
    on);
  - the head's pose while a carry layer plays (its bones could share the
    foreground's path to the head);
  - the base slot (flag 0x01 alone);
  - the animator's own carry clip (0x26).
- **Unverified:** the carry layer's fields against a capture. The tick
  stops before it ends, at Run Functional Tree.

Reopen if a capture shows the carry layer elsewhere in the skin's list,
or a carry clip that is not found and still plays.
