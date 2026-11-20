# Comparison harness

Runs the original game (`Sims.exe` 1.91 from the Complete Collection) under
Wine in an i386 QEMU guest, stops it at every lot update, and saves the state
we compare with our simulation. Step 4 of the roadmap's met goal (a headless
lot boot that matches the original); the current goal's recordings use it
too.

Nothing here holds game data. The inputs below are private and stay out of
the repository, and so do the captures.

## Inputs (private, not in the repository)

- The game ISO.
- An installed guest disk: an ext4 image of an i386 Debian with Wine, with the
  game installed in `/root/installer-prefix` and booting through
  `/sims-init.sh`.
- The guest's kernel and initramfs. The initramfs switches root to `/dev/vda`
  and runs `/sims-init.sh`.
- The install whose `UserData*` folders the game should read. They are the
  same folders our simulation reads, so the dump needs no other
  neighborhood file.

## Files

| File | What it does |
| --- | --- |
| `Dockerfile` | The `sims-harness-qemu:bookworm` image: Debian bookworm with `qemu-system-i386`, e2fsprogs and perl. |
| `prepare_guest.sh` | Makes a new guest directory. It clones the disk, copies the UserData folders and their hashes, and installs `guest/sims-init.sh`. |
| `guest/sims-init.sh` | PID 1 in the guest. It mounts the host share at `/research` and runs each `/research/cmd/<n>.sh` once. |
| `guest/prepare_disk.sh` | Replaces `/sims-init.sh` on the cloned disk, using debugfs inside the image. |
| `guest/launch.sh` | Guest job. It checks `Sims.exe` and `version.dll` against pinned SHA-256 digests (another build stops the launch) and writes the verified digests to the share (`game-identity.txt`), puts the UserData copy in place and checks its hashes, then starts Xvfb (1024x768x16) and `Sims.exe -w -nosound`. |
| `start_vm.sh` | Starts the guest in a container named `sims-harness-vm`: TCG, one CPU, 1 GiB, no network. The GDB stub and QMP listen only on the container's loopback. |
| `guest.py` | Drives the guest through the share: `run`, `shot` (PNG of the game window), `click` and `move`. Several processes can hand it jobs at once. `click` and `move` exit with the guest job's exit code, and `click` stops if the pointer cannot be placed. |
| `answer.sh` | Presses a dialog button and logs the press to the capture's `input-log.txt`. |
| `rsp.py` | A small GDB remote-protocol client, reached through `docker exec`. It reads memory and registers and sets hardware breakpoints. It never writes guest memory or registers. |
| `capture.py` | The capture: the breakpoints, the state readers and the output format. |
| `qmp.py` | A small QMP client, reached through `docker exec`, for memory dumps. It sends only `info mem` and `memsave`, which read the guest. |
| `qemu_retained_owners.py`, `qemu_neighborhood_owners.py` | The old prototype's readers for the registry and the neighborhood. |
| `input.py` | Scripted input for a capture (`--scenario`): the scenario format, the memory readers for the pointer, the pie menu and the people's queues, and the runner. |
| `guest/input-agent.sh` | Guest job that `input.py` starts. It runs the input commands the host drops in `/research/input`, in order, and polls often. |
| `test_input.py` | Tests of `input.py` against a fake guest memory. They need no Docker. |
| `manifest.py` | Writes `manifest.json` next to a capture: the updates and ticks it covers, which updates have a full state and why, the hours turned and the length of the game day, the objects that appeared or left, the dialogs and how each was answered, the presses and the queued actions. |
| `test_manifest.py` | Tests for `manifest.py` against a synthetic capture. |
| `test_capture.py` | Tests for `capture.py`, `guest.py` and `rsp.py` against a fake stub and fake memory, with no Docker: `python3 -B -m unittest discover -s tools/harness -p 'test_*.py'`. |

## Running a capture

```sh
docker build --platform linux/arm64 -t sims-harness-qemu:bookworm tools/harness
tools/harness/prepare_guest.sh <install> <installed guest ext4> <new guest dir>
tools/harness/start_vm.sh <guest dir> <game iso> <vmlinuz> <initramfs.cpio.gz>
python3 tools/harness/guest.py <guest dir>/share run 'sh /research/launch.sh' 900
```

The game reaches the neighborhood view in about 4 minutes. Check that it got
there with `guest.py <guest dir>/share shot view.png`.

Then start the capture in the background, before entering the house:

```sh
python3 -u tools/harness/capture.py --container sims-harness-vm \
    --output <new capture dir> --lot 5 --updates 1000 \
    --full 1-100,110-1000/10 --stop-file <stop file>
```

Enter the Goth house by holding the left button on it:

```sh
python3 tools/harness/guest.py <guest dir>/share click 658 214 15
```

The house loads in about 70 seconds. Answer the dialogs as they appear,
using `events.jsonl` (`dialog_show`) and a screenshot to tell when one is up:

- Update 1: five one-button notices. Press OK at about (400,378) or (400,400).
- Update 2: the Help System dialog. Press No at (490,378).

After that the household runs on its own.

The notices can close without a press. In one run (2026-11-14) only the
second notice was pressed; every notice and the Help System dialog still
logged a `dialog_result`, and the run went on as the others did.

Things to know:

- **Presses are handled late.** The game takes a press about 20 seconds after
  it is made. Wait for that dialog's `dialog_read` event before pressing
  again; otherwise a queued press lands on whatever comes next.
- **The neighborhood view runs the update too.** The lot update also runs
  there with lot 0. Those stops are only counted, as `entry_elsewhere`.
- **How long it takes.** A full state takes 4–7 seconds to read. An update
  that is not saved takes well under a second.
- **A state it cannot read stops the run.** The full state checks that
  every object a retained list names is in the registry. When one is not,
  the capture logs an `error` event, detaches and ends; the updates before
  it are kept.
- **The spatial-order cache can be stale.** The registry keeps a cache of
  objects in drawing order (registry +0x4c). Removing an object (0x44383c)
  only marks the cache dirty (0x443b65), and the draw rebuilds it before
  reading it (0x440349, 0x440397). So a dirty cache can name an object that
  is gone. The reader keeps a stale cache's pointers as read, with no
  identities (`stale`, `ids: null`), and checks identities only in a
  current cache. Before this, a run stopped at update 609 with `unknown
  retained spatial-order object identity`.
- **Sparse recordings.** For a long run (a whole game day is about 43,200
  updates) save the full state sparsely and around what happens:
  `--full 1-20,30-44000/10 --around 20 --before-hour 10`. `--around N`
  saves the update in which an object appears on or leaves the registry, a
  dialog is shown, or the hour turns, and the N updates after it
  (`objects_changed` and `hour_turned` events name what happened). The
  registry and the clock are compared from update 1's entry on, and every
  object's ID is read again at each return (an object made at a freed
  object's address is another object; the IDs of about 330 objects lie on
  about 80 pages).
  `--before-hour B` saves the last B updates before each hour turns. With
  either, every line of `updates.jsonl` also has the clock (`clock`:
  globals 0, 5 and 6), the date globals 1 to 3, 7 and 8 (`date`) and the
  registry's object count, and a saved update's line has `saved_for`. The
  random streams are on every line in any case. A full state takes about
  6 seconds and an update that is not saved about 0.16 seconds, so a day
  takes about 10 hours.
- **Dialogs during a long run.** `--answer-stuck 150 --guest-dir <guest dir>
  --window <window>` presses Return in the game window when a dialog shown
  in update `--answer-from` (default 3) or later has had no result for 150
  seconds, and again every 150 seconds, three times at most. Many notices
  close on their own (one at 7:00 did, in update 901 of the Goth house);
  this only keeps a run from waiting forever. Each press is logged
  (`dialog_auto_answer`, and `input-log.txt`). The dialogs are told apart
  by their record: a result ends its own dialog's presses, and a press job
  the guest has not run yet is cancelled (`dialog_auto_answer_cancelled`;
  the job checks its `.cancel` file right before it presses, and presses
  nothing if the game window cannot be focused). The result
  is `dialog_result`'s `result`, the function's argument: the stop is at
  its entry, where the record's `status` still holds the old value. Answer the first dialogs (updates 1 and 2) by hand as above:
  Return may not be the answer wanted there.
- **Manifest.** `python3 tools/harness/manifest.py <capture dir> --house
  'UserData/Houses/House05.iff (the Goths)' --note '...'` writes
  `manifest.json` from the capture's own files.
- **Stopping.** To end the capture, create the stop file. The capture then
  detaches, which resumes the game.
- **Taking over a running capture.** Kill `capture.py` and its `docker exec`
  relay while the guest is stopped, so the run never detaches. Then start
  `capture.py` again with the same arguments plus `--resume`. It continues the
  same directory and numbering, and redoes the update it was in the middle of
  (and a dump it was in the middle of). The events that update's return wrote
  are dropped first and written again (`resume_dropped`), when the run can
  write them again: an hour turn needs the last row's clock, an object
  change the registry of the last recorded return, which comes from
  `status.json` (`registry_unknown` if it is of another update). The
  dialogs still waiting for a result are taken up again, with the presses
  already handed to the guest (`resume_open_dialogs`).
- **Memory dumps.** `--dump 14,47 --guest-dir <guest dir>` also saves,
  at the entry of each listed lot update, every present user page of the
  game's process (`dump-NNNN/`; update 1's entry is the baseline's stop). QEMU writes each range into the guest
  directory (the container's `/g`), and the script moves it to the output.
  A dump holds the whole game, about 320 MB, so keep the list short. The
  dumps let a routine of the original be run alone on real memory.
- **Write watchpoints.** `--watch NAME=EXPR[:SIZE]` (repeatable) sets a
  hardware write watchpoint on SIZE bytes (1, 2, 4 or 8; default 4) and logs
  every write to them to `watch.jsonl`. EXPR starts with a hexadecimal
  address, `object:<id>` (the object with that ID) or `person:<id>` (the
  same, once it is a person), followed by steps: `+HEX` adds, `@HEX` reads
  the word at the address plus HEX. For example `dir=person:259@0xadc+0x88`
  watches the direction of person 259's animator. The watchpoint is set as
  soon as the address can be worked out. With `--watch` the lot update's
  entry and return stay armed, so each write can be placed inside or
  outside an update. Each logged write holds the writer's registers, its
  return addresses along the frame-pointer chain, the bytes before and
  after, and the update it falls in.

  Once the game's CR3 is known (at the first lot update), a write by another
  process to the same virtual address, including one found while stepping
  past a breakpoint, is logged as a `foreign_watch` event, not in
  `watch.jsonl`. A watch through an object or a pointer is worked out again
  at every stop in the game. If the address moved (an animator was
  replaced), the watchpoint moves too (`watch_moved`). If it cannot be worked
  out or read, it waits again (`watch_lost`). Writes between the change and
  that stop are missed. Watches on the same bytes share a watchpoint, and a
  write to them is logged once for each watch.

  QEMU's stop gives the base of the watchpoint that fired, not the bytes
  written. Which watchpoint fires, and whether it was written at all,
  depends on how QEMU split and checked the store. In QEMU 7.2 the newest
  watchpoint a check overlaps fires. An unaligned store on a watched page
  can be checked from the page's base. One instruction may be several
  stores. So each row's `match` says only what the stop and memory show:
  - `named`: the watch is on the watchpoint whose base the stop gives. It
    is `one_of` when several watchpoints share that base.
  - `changed`: the watch's value is not the one last seen. This store may
    have written it, or an earlier write that stopped nothing (through
    another mapping of the same memory). The new value is taken up, so a
    change is logged once.
  - `at`: the stop gives no watchpoint's base (QEMU never does this), and
    the watch holds the address.

  A watch that is not on the base the stop gives and kept its value has no
  row, even if the store wrote it the value it held: the stop does not
  show that.

  When a step over a breakpoint stops on a watchpoint, QEMU gives a second,
  plain stop at the next instruction on the following resume. A plain trap
  (`T05`, no watch) at the pc, CR3 and thread the step left is logged as
  `step_after_watch` and passed over; any other stop is dealt with as usual.
  Before the game's process is known (its first lot update), a stop whose
  lot cannot be read sets no watch, and a watch stop there is logged as
  `unknown_watch`, with no row and no value taken. Its memory may be
  another process's.

## Scripted input

`capture.py --scenario FILE --guest-dir <guest dir>` drives the game from a
scenario while it captures. The game only gets pointer moves, button presses
and keys, made with xdotool in the guest (QEMU's input devices do not reach
Xvfb). Each step's effect is checked in guest memory at a lot-update stop.
Nothing is written to guest memory.

A scenario has one step per line; `#` starts a comment:

```
answer-dialog 400 400          # wait for a dialog, press (400,400), wait for its result
until 20                       # wait until lot update 20 has returned
wait 10                        # let 10 more updates return
move 300 200                   # move the pointer
click 300 200                  # move, rest, press and release
key Escape                     # press a key (an xdotool key name)
click-object 173               # aim at object 173, click, keep the pie menu only if it is for 173
choose Sit                     # click the open pie menu's entry
expect-queued 21 173 within 100  # fail unless the menu pushed an action on person 21 aimed at 173
shot after                     # save the game window to input-shots/after.png (names are unique)
press 93 458                   # press a button of the game's panels (rest, move over it, press)
expect-mode buy within 300     # fail unless the lot's mode becomes buy (2); also live (0) or a number
expect-new-object within 600   # fail unless an object appears in the registry; logs it and the budget
```

For example, from a fresh boot (start the capture before entering the house,
as above):

```
answer-dialog 400 400
answer-dialog 400 378
answer-dialog 400 378
answer-dialog 400 400
answer-dialog 400 400
answer-dialog 490 378          # the Help System: No
wait 10
click-object 173               # the porch bench
choose Sit
expect-queued 21 173 within 100
```

And to buy a chair, after the same six dialogs and `wait 10`:

```
press 93 458                   # the buy-mode button
expect-mode buy within 300
press 258 524                  # Seating (after a fresh boot it opens on chairs)
wait 30
press 437 523                  # the catalog's first entry, the $80 Werkbunnst chair
wait 30
press 480 380                  # a free spot on the lawn
expect-new-object within 600
press 36 445                   # the live-mode button
expect-mode live within 300
```

How the checks work (the addresses are in `input.py`):

- **Aiming.** `click-object` puts the object's position through the house
  view's projection, moves the pointer above that point, and reads the ID of
  the object under the pointer from the picker tool. It presses when that
  reads the object three updates in a row. The game takes pointer moves
  late (tens of updates under TCG), so that reading can be stale. The pie
  menu that opens is the real check: if its actions are aimed at another
  object, Escape closes it and the next aim point is tried.
- **Pressing.** A press on the house view or a pie menu (`click`,
  `click-object`, `choose`) keeps the button down until three lot
  updates have returned, then lets it go. capture.py holds the game still
  for seconds at each stop, and a short press (down and up 0.3 s apart)
  that falls inside one stop is never seen: the game reads the button
  between frames. Twice such a press, with the picker on the object,
  opened no pie menu in 200 updates. The release is not waited on, so a
  press that opens a modal dialog does not hold up the steps after it.
  If the step fails or the capture ends while the button is down, it is
  let go then (a `command` row with a `reason`).
  `press` and `answer-dialog` hold the button 3 s, as `answer.sh` does.
- **Choosing.** `choose` reads the pie menu's items and their places on the
  screen (from the pie window the picker names at `+0xe4`), moves to the
  entry, waits until the menu shows it highlighted, and presses. A long menu
  is paged: item i is picker entry i + `+0x64`. An entry that opens a
  submenu (entry `+0x14` set; "Talk...", "Hug..." on a person) pushes
  nothing: the submenu opens in the people panel's other window, the parent
  window keeping its items. The step waits for the picker's window or
  entries to change, logs that `pie`, and the next `choose` picks from it.
  `choose @N` picks the N-th item (from 0), for a submenu whose labels are
  not known ahead; the logged pie names it.
- **Confirming.** capture.py stops at `0x48ddcb`, where the pie menu pushes
  the chosen action on the person's queue, and logs the person and the
  action. `expect-queued` waits for that record, then for the person to hold
  the action, in its queue (`+0x820`) or as its running action (`+0xa48`),
  and logs where it found it.
- **Dialogs.** A modal dialog holds the update it shows in, so
  `answer-dialog` runs on the dialog stops. It presses once, like
  `answer.sh`. If the game misses the press, the step waits until the dialog
  is answered some other way (for example with `answer.sh`).
  A step that waits on memory (`wait`, `until`, aiming, the pie menu's
  waits) waits on update returns, so a dialog the scenario did not open
  (the carpool, a visitor) holds it until the dialog is answered: run
  captures with `--answer-stuck` for those (it answers from update `--answer-from`, 3 by default). The menu may have closed by
  then; `choose` presses an item only while the menu shows it highlighted
  under the same label.
- **Buy mode.** `expect-mode` reads the lot's mode word (lot `+0x36`: 0 in
  live mode, 2 in buy mode). In buy mode the lot update still returns,
  without advancing time, so the steps go on there. `expect-new-object`
  compares the registry's object pointers with those at the scenario's
  first update return (or at the last `expect-new-object`), less the
  objects removed since, so an object that reuses a sold object's ID or
  memory still counts as new. It logs each new object (ID, type, position,
  level, and the definition's bytes and the data words as the full state
  has them) and the budget (lot `+0x90`) before and after.

A failed step is logged and ends the scenario; the capture goes on. A
dialog that opens while a step waits for update returns ends a command's
settling; the command's exit code (and a shot's conversion) is checked at a
later stop, and the scenario is done only when every command has ended (a
`scenario` row with `status` done, or unfinished if the capture ends first).
A choice that pushes an action and then opens a dialog ends at the dialog
stop. Steps that read memory wait for the next update return. A
scenario cannot be resumed: `--scenario` with `--resume` is refused.

`input.jsonl` is the record to replay. A `queued` row holds the update it
falls in: `update` N with `in_update` false means the push came before lot
update N, after update N-1 returned. Its `action` holds the caller, the
target, the routine, the priority, the menu number, the arguments and the
flags. The other rows log the steps, each guest command and when it ended,
the hover readings and the pie menus.

**Unverified.** The projection was checked only in the house's starting
view (zoom 1, no rotation). Submenus and build mode are not driven yet;
build mode's mode number is not known. The dialog, panel and catalog places
are fixed in the scenario, not read from the screen. What the catalog shows
depends on the panel's last state: after a fresh boot, Seating opens on
chairs. A press that lands on the wrong spot of the buy panel can sell an
object.

```sh
python3 -B -m unittest discover -s tools/harness -p 'test_*.py'
```

## Output

| Path | Contents |
| --- | --- |
| `entry-0000/` | The state at the entry of the first lot update on the lot. This is the state the load ends in, before tick 1. |
| `update-NNNN/` | The state at the end of lot update N, for the updates in `--full`. |
| `state.json.gz` | In both kinds of directory: the scheduler, the three random streams, the 64 globals, and every object with its frames (people with person data, motives and actions; doors, objects of type 8, with their portal costs as float bit patterns, `portal_bits`). |
| `full-state.json.gz` | In both kinds of directory: the registry, the neighborhood, and the object and animator pointers. |
| `pages.json.gz` | In both kinds of directory: the memory pages of the people and their animators. |
| `updates.jsonl` | One line per lot update: the tick, whether it was a full pass, the three random streams and the scheduler cursor; with `--around` or `--before-hour` also the clock, the date and the object count, and for a saved update why it was saved. |
| `events.jsonl` | The dialog functions passed (each record, its status and modal byte; a result also has `result`), the baseline, other stops, the game's identity (`game_identity`), the watch events, and with `--around` the objects that appeared or left (`objects_changed`), the hours turned (`hour_turned`) and the presses of `--answer-stuck` (`dialog_auto_answer`). |
| `dump-NNNN/` | With `--dump`: the user memory at the entry of lot update N. `ranges.json` lists the ranges (start, size, protection, file); `registers.json` holds the stop's registers and the stub's whole register packet. |
| `watch.jsonl` | With `--watch`: one line per write to a watched address (see above). |
| `status.json` | Progress, and `game_identity`: the digests `launch.sh` verified, or null if the game was not started by it. |
| `input-log.txt` | Every press made, with its time (written by `answer.sh`). |
| `input.jsonl` | With `--scenario`: the steps, the guest commands, the pie menus and every action the pie menu queued, each with its lot update (see Scripted input). |
| `input-shots/` | With `--scenario`: the `shot` steps' screenshots. |
| `manifest.json` | Written by `manifest.py` (see above). |
