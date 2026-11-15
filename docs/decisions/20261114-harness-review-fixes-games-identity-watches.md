# Harness review fixes: the game's identity, watches and pages

*Decided 2026-11-14.*

A review of `tools/harness` found eight defects in the harness, all fixed
here, with tests that need no Docker (`tools/harness/test_capture.py`). A
finding in `scripts/check_repository.py` is fixed separately.

- **The game is pinned.** `guest/launch.sh` checks `Sims.exe` and
  `version.dll` against their SHA-256 digests and stops for any other build.
  It writes the verified digests to the share, and `capture.py` records them
  in `status.json` and as a `game_identity` event. Captures made before this,
  or of a game started some other way, show it as null. Reopen if the harness
  has to run another build.
- **Writes by other processes are not the game's.** QEMU's watchpoints are
  on virtual addresses, so another process writing the same address stops
  the guest too. Once the game's CR3 is known, such a write is a
  `foreign_watch` event, not a row in `watch.jsonl`. This includes a write
  found while stepping past a breakpoint. Before the first lot update the
  game's CR3 is not known, so early writes are still not checked.
- **Watches follow their pointers.** A watch through an object or a pointer
  is worked out again at every stop in the game. When an animator is
  replaced, the watchpoint moves to the new one (`watch_moved`), or waits if
  there is none (`watch_lost`). Writes between the replacement and that stop
  are missed. A watchpoint set at each change of the pointer itself would
  catch them; that is not done. Watches that land on the same bytes share
  one hardware watchpoint, which stays until the last of them leaves.
- **Pages.** An animator or layer that ends on a page boundary made the
  snapshot ask for the next page, which it had not read, and the capture
  ended with a `KeyError`. The last page is now that of the last byte.
  `pages.json.gz` no longer holds a page past the end of such a block.
- **`--dump 1` works.** Update 1's entry is the baseline's stop, which never
  dumped. A run taken over with `--resume` in the middle of a dump redoes it
  (a dump is complete once its `registers.json` is written).
- **The stub's checks hold under `python -O`.** The packet checksum and the
  allowlists that keep `rsp.py` and `qmp.py` read-only were asserts. They are
  now explicit checks.
- **Guest jobs.** `guest.py` reserves each job number by creating its file
  exclusively, so two processes no longer take the same number and lose a
  job. `click` and `move` pass on the guest job's exit code, and `click`
  stops if the pointer cannot be placed.
