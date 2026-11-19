# Change Suit keeps an unnamed accessory when the suit lookup fails

*Decided 2026-11-18.*

The person primitive (0x45d962) passes the lowered name and an empty
binding to the animator's dress function (0x58d88f). Dress calls undress
with the requested name, then appends a new entry to the animator's
list (+0x274, append at 0x58d8ec). The constructor (0x583479) sets the
attachment pointer to null and makes both the name (+4) and binding
(+0x10) empty. The binder (0x583559) fills those fields only after
attaching the suit. A lookup miss (0x5a044d), including a lazy load
that leaves no suit (0x5a38da), returns before any of those writes.
Dress leaves the entry in place and the primitive branches true.

Undress (0x58d919) compares the requested name with the stored name by
length and bytes (0x431726). A later undress of the requested name
leaves the empty entry in place. A second failed dress of the same
name appends another empty entry. The primitive rejects an empty
requested name with native error 0x30 (0x45dad3). The native animation
event passes its string directly to undress (0x591481), so an event
with an empty string would match all the empty entries.

The animator saves the full list count and each name and binding in
order (0x591b59 to 0x591c01). Loading rebuilds one entry per pair.
When loading or rebinding the list (0x588d6a), an empty name returns
from the binder before the lookup (0x583569). The routine that copies
the list before rebuilding the avatar also copies empty pairs
(0x5930c4, 0x593478). The list reads traced here are list maintenance,
appearance and saving. An empty entry supplies no attachment.

The port uses `People::suits` for the lookup and appends an `Accessory`
with two empty strings on a miss. The existing `Person::accessories`
encoding preserves their count and order in saves. The renderer's body
cache sees the full list, and its outfit reader skips empty names.
Synthetic tests cover replacement of a named entry on a miss, repeated
misses, undress, and snapshot decoding.
This supersedes the first port's decision to stop on a miss.
[Change Suit/Accessory, and K = 408](20261114-change-suit-accessory-k-408.md)

**Unverified.** The branch and save behavior are read from the code.
The Goth capture shows an accessory count and lacks the entries. A
recording of a failed lookup and a save with the empty entry would
check the reading. The original registry's population and lazy loads
are still outside the port, so membership in the decoded suit set is
the same approximation used for successful binds. The general
primitive table and the animation events that call dress and undress
(0x591435, 0x591481) remain outside this change.
