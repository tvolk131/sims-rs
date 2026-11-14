# Comparison harness

Runs the original game (`Sims.exe` 1.91 from the Complete Collection) under
Wine in an i386 QEMU guest, stops it at every lot update, and saves the state
we compare with our simulation. Roadmap step 4.

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
- **Stopping.** To end the capture, create the stop file. The capture then
  detaches, which resumes the game.
- **Taking over a running capture.** Kill `capture.py` and its `docker exec`
  relay while the guest is stopped, so the run never detaches. Then start
  `capture.py` again with the same arguments plus `--resume`. It continues the
  same directory and numbering, and redoes the update it was in the middle of
  (and a dump it was in the middle of).
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

## Output

| Path | Contents |
| --- | --- |
| `entry-0000/` | The state at the entry of the first lot update on the lot. This is the state the load ends in, before tick 1. |
| `update-NNNN/` | The state at the end of lot update N, for the updates in `--full`. |
| `state.json.gz` | In both kinds of directory: the scheduler, the three random streams, the 64 globals, and every object with its frames (people with person data, motives and actions; doors, objects of type 8, with their portal costs as float bit patterns, `portal_bits`). |
| `full-state.json.gz` | In both kinds of directory: the registry, the neighborhood, and the object and animator pointers. |
| `pages.json.gz` | In both kinds of directory: the memory pages of the people and their animators. |
| `updates.jsonl` | One line per lot update: the tick, whether it was a full pass, the three random streams and the scheduler cursor. |
| `events.jsonl` | The dialog functions passed (each record, its status and modal byte), the baseline, other stops, the game's identity (`game_identity`) and the watch events. |
| `dump-NNNN/` | With `--dump`: the user memory at the entry of lot update N. `ranges.json` lists the ranges (start, size, protection, file); `registers.json` holds the stop's registers and the stub's whole register packet. |
| `watch.jsonl` | With `--watch`: one line per write to a watched address (see above). |
| `status.json` | Progress, and `game_identity`: the digests `launch.sh` verified, or null if the game was not started by it. |
| `input-log.txt` | Every press made, with its time (written by `answer.sh`). |
