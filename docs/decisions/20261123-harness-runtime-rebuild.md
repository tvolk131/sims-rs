# The harness kernel and initramfs are built from the guest disk

*Decided 2026-11-23.*

A disk cleanup on 2026-11-23 deleted the private directory that held the
comparison harness's kernel, initramfs and seed guest disk. There was no
backup, and nothing in the repository could make them again. Every harness
VM boots from those two files, so no new capture could start.

What was decided:

- **The kernel and initramfs come from the installed guest disk.**
  `tools/harness/build_runtime.sh <installed guest ext4> <new out dir>` reads the
  disk with debugfs inside the harness image. It works on an APFS clone
  with the journal replayed, as `prepare_guest.sh` does for a guest, so it
  reads what a prepared guest boots and never mounts the disk itself. It
  copies `/boot/vmlinuz-<version>` and builds the initramfs
  from the disk's own static busybox and modules. The kernel, the modules
  and the disk then always match. The script needs only the disk and the
  image (no download, no `cpio` package: `runtime/newc.pl` packs the archive
  with perl-base). The same disk gives the same files byte for byte. The
  output directory appears whole or not at all.
- **The initramfs does what the old one did.** The old `/init` was found in
  a session log of 2026-11-14 and is kept as `runtime/init`. It loads
  virtio_pci, virtio_blk, crc32c_generic and ext4, mounts `/dev/vda`, moves
  `/dev`, `/proc` and `/sys` onto it and switch_roots into `/sims-init.sh`.
  There are two differences. The old archive carried the whole module tree
  (73 MB). The new one carries only those four modules and what
  `modules.dep` says they need, eleven in all (1.5 MB). The 9p and isofs
  modules were always loaded after the switch, from the disk. The new
  archive also has a `/dev/console` node, so `/init`'s errors reach the
  serial log.
- **The kernel is the same build.** The kept disk's
  `/boot/vmlinuz-6.1.0-53-686` is 5,711,808 bytes, the size of the lost
  file, and comes from the same Debian package version. The lost file's
  digest was never recorded, so byte identity cannot be checked.
- **The seed disk is a clone of the first harness guest's disk.** The
  original seed (`installer-guest.ext4`, 2026-10-20) is gone. The new seed
  is an APFS clone of `sims-harness-guest-20261114-first/working.ext4`,
  kept read-only. That disk has been booted and played, so it differs from
  the old seed:
  - its last VM was stopped while the disk was mounted, so the journal
    needs recovery (the `e2fsck -fy` in `prepare_disk.sh` replays it on
    the clone);
  - Wine rewrote `system.reg`, `user.reg`, `userdef.reg` and the
    `dosdevices` COM links in the prefix (last on 2026-11-21);
  - `/tmp` holds an X lock (`launch.sh` removes it) and the input agent's
    lock file;
  - the game's `UserData*` folders are from the last run, but `launch.sh`
    replaces them and checks their hashes before every start.

  `Sims.exe` and `version.dll` still pass `launch.sh`'s digest check.
  **Unverified:** that none of the Wine prefix changes alter what the
  game does. A new capture of the Goth house compared with an old one
  would settle it.
- **`prepare_guest.sh` makes its clone writable.** `cp -c` keeps the
  seed's mode, so a read-only seed gave a read-only guest disk and
  `prepare_disk.sh` failed.

How it was checked: a guest prepared from the new seed booted on the new
kernel and initramfs, wrote `SIMS_HARNESS_GUEST_READY` to the serial log
about ten seconds after start, ran `launch.sh` (identity and UserData
checks passed) and reached the neighborhood view.

What would reopen this: a capture on the rebuilt runtime that differs from
an older capture where the code has not changed, or a guest disk whose
kernel needs modules that `runtime/init` does not load.
