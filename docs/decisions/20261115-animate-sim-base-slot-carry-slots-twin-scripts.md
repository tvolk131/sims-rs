# Animate Sim on the base slot is the carry slot's twin; a script's base clip stays until the posture changes

*Decided 2026-11-15.*

**Unverified.**

Animate Sim with flag bit 0x01 alone (0x58957e; the slot picker 0x47ec4c
returns 1) works on the base slot (animator +0x14) the way flag 0x10
works on the carry slot. It never yields and always takes the true branch
(0x589c4e). Clip 0 clears the slot (virtual function 0x104, 0x592878).
The base slot's layer is always removed at once, even where the other
slots could fade (0x592890). Any other clip is looked up (virtual
function 0x12c, 0x5893b3), the slow flag (+0xa8) set from flag 0x80, and
the clip started in the base slot (virtual function 0x100, 0x592726),
backward with flag 0x02, to repeat at priority -10 (0x59867f). That is
what the posture's own base clip gets. Unlike the carry slot, no mode or
selection is written.

The base slot used to be a flag, since only the posture's clip went
there. It is now `Base`: empty, the posture's clip, or a clip a script
started, with its name, direction and the animator's clock at its
start. Drawing plays it from that start (0x597a88: the layer's clock
starts at the animator's, its phase at 0, or 1 backward), not from the
clock's zero. `Empty` and `Posture` encode as
`false` and `true` did, so existing states and snapshots hash and decode
as before. The animator's base update (0x590430) treats a script's layer
like any other: a change of posture clears it. Otherwise it stays for a
person of kind (word 65) at most 1, and for any other kind the posture's
clip replaces it, unless bit 2 of definition word 102 is set. Animate Sim
does not touch the posture noted (+0x198), so after clip 0 the next base
update starts the posture's clip again. The head's measurement checks a
script's base clip against the foreground as it checks the posture's.

Beyond a script's start, the base layer's clock and its place in the
skin's list are still not kept. A new layer goes in before the first
layer above priority 1 (0x599128) and only then takes -10, so it may
come before or after the foreground layer; the skin's update sorts its
layers by priority first (0x5991fd), so the base updates first anyway. That matters only for timed events, so a script's base
clip with timed events fails loudly (0x5992b3), as the posture's does.
The original goes on past a clip it cannot find (0x452bed reports it);
here that fails, as it does for the other slots.

**On Goth.** The social person 16 picks clears object 16's base slot
(clip 0, source byte 2, which clip 0 never reads) in the next update.
The lot then runs 46 updates and stops at tick 48 on object 21, tree
0x2025 node 13, Generic Sims Call routine 15 (0x44fe3c). Strict mode on
the Goth house is unchanged. Reopen when the harness records a social,
or when a script plays a base clip with timed events.
