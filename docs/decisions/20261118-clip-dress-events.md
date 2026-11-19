# A clip's dress and undress events put accessories on and take them off

*Decided 2026-11-18.*

A clip's timed event with the key "dress" or "undress" (0x5912fd) calls
the person animator's virtual function 0xd8 or 0xdc with the event's value:

- **"dress"** (0x59140b to 0x591453): the value, with the empty string at
  0x6de5b0 as the binding, goes to 0xd8 (0x58d88f).
- **"undress"** (0x591469 to 0x59148e): the value goes to 0xdc (0x58d919).

These are the routines Change Suit/Accessory uses, so the port shares
them. Dressing first takes off any accessory of the same name, then adds
one at the end with an empty binding. If no avatar file defines the name,
the new entry has no name. Undressing takes off every exact match.

**What differs from Change Suit.** Change Suit lowercases the name
(0x46f8f7). The event passes its value as written. The avatar registry's
keys are lowercase (0x5950f5). Whether its lookup (0x5a044d, through
0x5a25dc) ignores case is not worked out. All 131 distinct dress and
undress values in the shipped animations are lowercase, so a value with an
ASCII capital fails loudly, naming 0x5a044d.

**Effect.** House 69 of the first neighborhood (the expansion lot,
`20261118-expansion-lot-house-69.md`) stopped at tick 495 on Claire's
"a2o-petbowl-fill". With this change and the house's GUID table read, a
strict run reaches tick 1000 when every dialog is answered with its first
button. Dialogs come at ticks 1, 3 and 101. No base-game clip the Goth run
reaches has such an event (the run would have failed), so the strict
Goth run is unchanged.

**Unverified:** that the registry the original consults holds the names
in `People::suits`. Change Suit's entry already notes this.
