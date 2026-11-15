# Buy mode is driven by panel presses and confirmed by the mode, the registry and the budget

*Decided 2026-11-14.*

Scenarios can now use buy mode (`tools/harness/input.py`). There are three
new steps: `press X Y`, `expect-mode` and `expect-new-object`.

- **Presses on the game's panels work like dialog presses.** The pointer
  rests, moves over the button, then presses. A plain click was taken late
  or missed, as on dialogs.
- **What is checked.** The mode word is lot `+0x36`, read in the running
  game: 0 in live mode and 2 in buy mode. A purchase is confirmed by new IDs
  in the lot's registry and by the budget (lot `+0x90`). In buy mode the lot
  update still returns, with time not advancing, so the steps go on there.
- **Checked in the original, from a fresh boot.** The scenario:
  - entered buy mode (mode 2 at update 174);
  - pressed Seating and took the catalog's first entry, the $80 Werkbunnst
    chair;
  - pressed a free lawn spot;
  - went back to live mode (mode 0 at update 1078).

  At update 878 the registry held a new object, ID 262, at (344, 632) in
  sixteenths of a tile, level 1. The budget had gone from 2542 to 2462. The
  screenshots show the chair on the lawn and $2,462.
- **Unverified.**
  - A second new object, ID 292 at (-16, -16), off the lot, appeared by the
    same update. Whether the purchase made it is not checked.
  - Build mode's number is not known.
  - The catalog's contents depend on the panel's last state: after a fresh
    boot, Seating opens on chairs, and in an earlier session it showed beds.
    A press on an empty spot of the panel once sold an object. Scenarios
    must start from a known panel state (a fresh boot).
