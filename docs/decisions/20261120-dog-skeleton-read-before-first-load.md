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
- The port's load drew 6,497. The dog is object 9 and the adult
  (Claire) object 274. The dog file costs 1,776 draws: its skeleton,
  then the dog's copy. Read before the load, the dog builds only its
  copy, and the adult's default-trait draws change from 33 to 42, so the
  load draws 1,767 fewer.
- With the dog file read before the load, the port draws 4,730, and the
  stream, the lot and global streams and every object's state equal the
  recording's at the entry, except for two load-time differences that
  this does not touch (a 0x8000 bit on four objects' frame trees, and
  one object's last result).

**The Goth day changes too.** At strict tick 10,358 the Goth house makes
its first dog. Before this change that read the dog file there and drew
3,552; now it draws 1,776, its own copy. The original's recording
(`goth-day-20261115`) draws 1,776 for the same dog at update 24,766.

**Consequences.** The list is in the World, and the first dog's draws
change, so the strict-boot pins change (44 of them, TICK_43200 among
them). With the old list and C runtime state restored, all 44 old values
come back. A snapshot saved before this change still decodes, but
restoring it fails with `SaveError::OtherLoad`.

**Where and in what order.** The avatar registry's scan at startup
(0x59f844) reads the files through 0x59f087, whose skeleton records go
through 0x598af7. On the Complete Collection the file map's walk reaches
the dog file before the kat file. The list now names them in that order,
although the World keeps them as a set, so the order has no effect.

**Unverified.** The effects1 skeleton in the same folder: no capture has
made one, so it is not listed. That the scan reads them before the first load,
and not between the load and the first dog, is unverified.
