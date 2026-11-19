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

**The name.** Change Suit lowercases its name (0x46f8f7). An event's
value was already lowercased when the clip was read (0x59431a, 0x594390,
0x63822a; `Track::build`). The suit lookup (0x5a044d) compares bytes
exactly: 0x5a25dc hashes the bytes (0x5b8ef9), then checks length and
bytes (0x5a27f2, 0x5a280f). The registry's keys are lowercase (0x5950f5).
An empty value is not looked up (0x583569). It takes off any unnamed
entries and adds a new unnamed one, so it never trips the check for
avatar files that stopped decoding. All 131 distinct dress and undress
values in the shipped animations are lowercase and non-empty.

**Other animators.** An ordinary object's animator also reaches 0x5912fd
(its slots call 0x5857e7 and 0x585878). The port rejects such objects
earlier, at Animate Sim. If one got here, the event would be native error
0x1c.

**Effect.** House 69 of the first neighborhood (the expansion lot,
`20261118-expansion-lot-house-69.md`) stopped at tick 495 on Claire's
"a2o-petbowl-fill". With this change and the house's GUID table read, a
strict run reaches tick 1000 when every dialog is answered with its first
button. Dialogs come at ticks 1, 3 and 101. No base-game clip the Goth run
reaches has such an event (the run would have failed), so the strict
Goth run is unchanged.

**Unverified:** that the registry the original consults holds the names
in `People::suits`. Change Suit's entry already notes this.

Reviewed by GPT-6.2 Sol, which confirmed the dispatch (the `SAnimator`
table at 0x671ee4), the arguments, the case-sensitive lookup and the
archive census.
