# The comparison harness runs again, and a capture holds 1000 ticks

*Decided 2026-11-14.*

The original runs again in the emulator and can be stopped and read at
every lot update. `tools/harness/` holds the scripts. A new private
capture of the Goth house holds the state at the entry of tick 1, after
each of the first 100 lot updates, and after every tenth up to 1000.

What was decided:

- **The harness is the old prototype's, cut down.** Debian bookworm
  with `qemu-system-i386` in Docker. An i386 guest with full emulation
  (TCG) runs `Sims.exe -w -nosound` under Wine. Hardware breakpoints
  stop the game at the entry of the lot update (0x46d345) and at the
  end of its body (0x46d51d). State is read through QEMU's GDB stub.
  Nothing writes guest memory or registers. The only inputs are the
  button presses the README lists.
- **The guest reads the install's own UserData.** The guest's UserData
  folders are replaced with copies of the install's, and their hashes
  are checked before the game starts. With that, the C runtime stream
  and all 463 neighborhood records match ours at the end of the load.
  The first capture's 52-draw offset and its five differing records
  came from that guest's own files, not from the code.
- **Dialogs are answered as in the first capture.** OK to the five
  notices in tick 1 (read with status 2), and No to the Help System in
  tick 2 (status 3). The game closed two later dialogs itself, in
  updates 901 and 902.
- **Captures stay private.** They live in the old checkout's ignored
  `cache/`. The guest disk, kernel, initramfs and ISO are inputs that
  the repository does not hold.

How it was checked: our state at the entry of tick 1 and after each of
ticks 1 to 5 was compared with the new capture field by field. 248
lines are equal and 3 differ:

- Registry byte 0x12 is already 1 when the original's load ends. Ours
  sets it in tick 2. This is two of the lines (load and tick 1).
- After tick 1, global 12 is 17 in the original and 0 in ours. Global
  12 is the interface's wait check, which we do not run. The first
  capture was read at the entry of the next lot update, not at the end
  of tick 1, and had 0 there. **Unverified:** that the open dialog sets
  it and something outside the lot update clears it.

Known gaps in the capture: the animator pointers were not recorded for
updates 1 to 45. For ticks 1 to 5 the comparison uses the pointers
recorded at update 46, on the same pages.

What would reopen this: a capture whose load does not match ours from
the install's files, or a need to observe something the GDB stub
cannot reach.
