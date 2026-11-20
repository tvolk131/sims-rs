# The dog skeleton file is read before the first load

*Decided 2026-11-20.*

`sims_vm::READ_BEFORE_THE_FIRST_LOAD` now lists the dog skeleton file
(`ExpansionShared.far`, `Skeletons\dog_skeleton.cmx.bcf`) as well as the
kat file (see `20261118-kat-skeleton-read-before-first-load.md`).

**The recording.** Track 2 recorded the first neighborhood's House 69
(`house69-ud1-ticks-20261119`, entered from a fresh start on the
neighborhood view). The house holds a dog.
- The C runtime's stream at the entry of the first lot update is
  2,984,970,331: 4,730 draws on from its initial state 1.
- The port's load drew 6,497. The adult person (object 9) read the
  adult file and made the copies, and the dog (object 274) read the dog
  file and made its own copy. The 1,767 extra draws are the dog file's
  skeletons.
- With the dog file read before the load, the port draws 4,730, and the
  stream, the lot and global streams and every object's state equal the
  recording's at the entry, except for two load-time differences that
  this does not touch (a 0x8000 bit on four objects' frame trees, and
  one object's last result).

**Consequences.** The list is in the World, so the strict-boot pins
change, as they did for the kat file. A snapshot saved before this change
still decodes, but restoring it fails with `SaveError::OtherLoad`.

**Unverified.** The effects1 skeleton in the same folder: no capture has
made one, so it is not listed. Where the original reads these files is
still unverified (likely the avatar registry's scan at startup, 0x59f844).
