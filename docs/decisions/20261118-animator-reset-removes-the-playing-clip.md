# Animator reset removes the playing clip

*Decided 2026-11-18.*

The person's animator reset (virtual function 0xc, 0x589220) calls
0x586b63 at 0x5892ea. That function clears the foreground slot (+0xc)
through virtual function 0x104 (0x592878) and sets the selection (+0x98)
to -1, including when the slot was empty. The reset then clears the
blend (+0x10), carry (+0x13c) and base (+0x14) slots in that order. At
0x58931d it empties the numbered event queue (+0x2c) through 0x407161,
then clears the playing flag (+0xa0) at 0x58932c. Clearing the foreground
slot itself makes no change to the queue or the playing flag.

**Immediate removal.** For an occupied slot the direct branch at
0x5928f2 calls 0x5a0a7b, which forwards to the skin's 0x5991a0. It finds
the layer in the skin's list (+0x7c), marks the layers changed (+0x94)
at 0x5991d4, erases the list entry through 0x59b0bd, and destroys and
frees the layer. 0x5928fb then zeros the slot pointer. An empty slot
returns before touching the skin. (A layer not found in the skin's list
would be dropped from the slot without marking the skin changed; a
slot's layer is always in the list here, so the port does not keep that
case.)

**Fade gate.** A layer fades only if the byte at 0x6f6bd0 and the
duration at 0x6d0460 are both nonzero and the slot is other than the
base. The installed Complete Collection PE places 0x6f6bd0 in the
zero-filled tail of its data section. In the mapped executable image
its only absolute references are the compares at 0x586cb2 (the base
animator's clear) and 0x592887 (the person's animator's clear). The
byte is 0 in the memory images of the running game taken at updates
14, 15, 47 and 580, and the duration is 200. No direct writer or
reference taking the byte's address was found.

With the gate enabled, 0x5928a3 and 0x5928a7 set two layer fields
(+0x7c and +0x78) to -1. 0x597a64 sets the fade flag and records the
current clock, duration, current weight and target weight zero.
Foreground removal first removes any previous blend layer, moves the
foreground layer into the blend slot, and increments its priority
through 0x59867f, marking the skin changed. A clear of another eligible
slot sets the layer's +0x64 retirement flag. The reset then clears the
blend and carry slots, leaving the outgoing foreground layer and any
carry layer detached with that flag set. The base always leaves the
skin immediately.

`Animator::clear_foreground` now implements the immediate branch for
both reset and the VM callers of 0x586b63. The reset discards the
foreground even if the playing flag is clear, as it can be for a walk.
The blend slot stays empty in the supported executable, so the reset
needs no fade state or refusal.

**Runs past the reset in strict mode.** Strict mode only shows that no
unported path is reached. In the portrait-and-fridge-order test, the
salesman leaves at tick 510 and the lot runs on (the test checks to
tick 540; it was run to 3540 without a stop). In the pie-menu social
test, the salesman is deleted at tick 597 with `A2O-door-knock` in the
foreground, where the run used to stop; the test checks that he and his
animator are gone then, and runs to tick 625 (run to 3625 without a
stop). Neither run was compared with a recording here. The Goth day
under the recording's frames used to stop at tick 16112, where object
26's Burn resets person 18's animator with `A2O-panic-loop2` playing; it
now runs to 17928 and stops at 17929 on Generic Sims Call routine 6.
*Corrected 2026-11-19:* the recording does not end at 16111; it runs to
update 44200. The lot and C runtime streams match it at 16112, 17562
and 17928, pinned in `boot.rs`
(`docs/decisions/20261119-generic-sims-call-6-and-the-saved-neighbor-record.md`).

**Unverified.** The reference scan covers the mapped Complete Collection
executable and direct addresses. A write through an indirect address
or by a different executable has not been ruled out. The faded layers'
later poses and events under an externally enabled gate were not
compared with the original.
