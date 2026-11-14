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
| `guest/launch.sh` | Guest job. It checks the executable's hash, puts the UserData copy in place and checks its hashes, then starts Xvfb (1024x768x16) and `Sims.exe -w -nosound`. |
| `start_vm.sh` | Starts the guest in a container named `sims-harness-vm`: TCG, one CPU, 1 GiB, no network. The GDB stub and QMP listen only on the container's loopback. |
| `guest.py` | Drives the guest through the share: `run`, `shot` (PNG of the game window), `click` and `move`. |
| `answer.sh` | Presses a dialog button and logs the press to the capture's `input-log.txt`. |
| `rsp.py` | A small GDB remote-protocol client, reached through `docker exec`. It reads memory and registers and sets hardware breakpoints. It never writes guest memory or registers. |
| `capture.py` | The capture: the breakpoints, the state readers and the output format. |
| `qemu_retained_owners.py`, `qemu_neighborhood_owners.py` | The old prototype's readers for the registry and the neighborhood. |

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

Things to know:

- **Presses are handled late.** The game takes a press about 20 seconds after
  it is made. Wait for that dialog's `dialog_read` event before pressing
  again; otherwise a queued press lands on whatever comes next.
- **The neighborhood view runs the update too.** The lot update also runs
  there with lot 0. Those stops are only counted, as `entry_elsewhere`.
- **How long it takes.** A full state takes 4–7 seconds to read. An update
  that is not saved takes well under a second.
- **Stopping.** To end the capture, create the stop file. The capture then
  detaches, which resumes the game.
- **Taking over a running capture.** Kill `capture.py` and its `docker exec`
  relay while the guest is stopped, so the run never detaches. Then start
  `capture.py` again with the same arguments plus `--resume`. It continues the
  same directory and numbering, and redoes the update it was in the middle of.

## Output

| Path | Contents |
| --- | --- |
| `entry-0000/` | The state at the entry of the first lot update on the lot. This is the state the load ends in, before tick 1. |
| `update-NNNN/` | The state at the end of lot update N, for the updates in `--full`. |
| `state.json.gz` | In both kinds of directory: the scheduler, the three random streams, the 64 globals, and every object with its frames (people with person data, motives and actions). |
| `full-state.json.gz` | In both kinds of directory: the registry, the neighborhood, and the object and animator pointers. |
| `pages.json.gz` | In both kinds of directory: the memory pages of the people and their animators. |
| `updates.jsonl` | One line per lot update: the tick, whether it was a full pass, the three random streams and the scheduler cursor. |
| `events.jsonl` | The dialog functions passed (each record, its status and modal byte), the baseline, and other stops. |
| `status.json` | Progress. |
| `input-log.txt` | Every press made, with its time (written by `answer.sh`). |
