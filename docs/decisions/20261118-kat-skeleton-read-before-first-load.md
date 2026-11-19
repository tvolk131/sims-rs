# The kat skeleton file is read before the first load

*Decided 2026-11-18.*

Building a person's skeleton draws from the C runtime's random stream
(0x636c74): a file read for the first time builds every skeleton in it
(0x598af7, from the reader 0x59f087 at 0x59f107), and then the person
gets its own copy (0x598da3). The port keeps the files read in
`World::skeleton_files`, and until now took it that no file was read
before the first load.

**The recording.** In `goth-day-20261115` a cat is made at updates 6756,
10358, 17562 and 24719, and each time the C runtime's stream moves by
1,920 draws, one skeleton's worth. The port made the first cat read the
kat file there and took twice that at 6756. The stream at the entry of
the first lot update is 7,212 draws on from its initial state 1, which
the load's own reads (the adult and child files and the three Goths)
account for, so the kat file was read before, without moving that
stream. The C runtime keeps its state per thread ([_getptd]+0x14, read
at 0x63e2b3; `_getptd` is 0x30f00d0 in all 17 captures, so the state is
at 0x30f00e4), so a read on another thread, or before `srand`
(0x619e5a seeds it once from rdtsc), does not show.

**Decision.** `sims_vm::objects::READ_BEFORE_THE_FIRST_LOAD` lists the
kat file, and the boot puts it into `skeleton_files` before the house is
read. With it the lot and C runtime streams match the recording at 6756
and on to 16111 (under the recording's frames; see
`20261118-goth-day-frame-schedule.md`). No World field is added and the
snapshot format does not change, but the list is in the World, so every
strict-boot pin changes, from tick 33 on, in each format the tests pin.

**Unverified.** Where the original reads it: the avatar registry's scan
at startup (0x59f844, called from 0x5843b6 and 0x587194) calls the
reader 0x59f087 (at 0x59fa98), which is the likely place. The same
folder of `ExpansionShared.far` holds `dog_skeleton` and
`effects1-skeleton` (and `AnimationOut.ndx`), which may be read then
too; no capture has made a dog or an effect skeleton.

**What would reopen it.** A dog made in a capture: if its first one
moves the stream by two skeletons, the dog file is not read before the
load.
