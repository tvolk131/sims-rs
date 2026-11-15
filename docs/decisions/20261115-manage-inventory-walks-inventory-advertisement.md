# Manage Inventory walks an inventory; the advertisement record starts zeroed; scope 24 fields 0 and 1

*Decided 2026-11-15.*

Track 1's pie menus on the Goth house failed in check trees on these.
Listing every object's menu on the loaded house, 9 failed: 5 on Manage
Inventory operation 4, 1 on scope 12 and 3 on scope 24 field 0. Now 6
list. The other 3 (the phones) go on to scope 24 field 1, which is now
ported too, and then stop at Add/Change Action String's formatter
(0x44bf1a) on "Call Cab.../Go to Magic Town - $$Local:1", which is
track 1's.

**Manage Inventory operation 4** (0x455462, with 0x41f69c and 0x41f746)
finds the next token of a kind after the place in the caller's temp
`(flags >> 8) & 3`. With flag 0x1 it compares the GUID too; without it,
any GUID matches. Temp `(flags >> 5) & 3` takes the count and then temp
`(flags >> 11) & 3` the place. With none they take 0 and -1 and the
branch is false. Unlike operation 3, a token with a count of 0 is found.
The search starts at the place plus one as a word, so after 0x7fff it
wraps negative and finds nothing. The place written is the token's index
cut to a word, with no check (0x41f6d5 to 0x41f6e1; operation 3's
0x41f677 to 0x41f684 likewise, where this used to fail). A token at
index 65535 is written as place -1, and the branch, which tests the
place, is false. As for every operation, the owner's
inventory is made if it has none, and with GUID 0 the stack object's is
read first, whether or not the flag uses it. Setting the interface's
inventory flag on success is presentation.

**The advertisement record before the first copy.** The earlier entry [20261115-advertisement-record-outlives-find-best](20261115-advertisement-record-outlives-find-best.md)
left it unread, failing loudly. It is now ported from the code: the
record's constructor (0x475e23) allocates its sixteen advertisements and
zeroes each one, and it runs from the static initializer 0x458271, which
is in the start-up table at 0x6c0154. Nothing else references the
record: 0x6e5100 appears only in that initializer, its destructor and
the pointer 0x6c437c. The pointer is read only by the three scopes and
the copy 0x4763c5. So until the first copy, scopes 12, 28 and 29 read 0,
and a copy of fewer than sixteen leaves zeros in the rest. In the World,
`advertisement_record` stays None until something writes it, and None
stands for those zeros, so the encoding and every hash are unchanged.
This holds for a game that has not run a lot since it started, as every
capture has.

**Scope 24 field 0** (0x425baa) is the ID (object +0xf0) of the first
person in the lot's list of people (lot +0x84, `World::people`) whose
definition (+0x11c) is the one the record is linked to (+8). It is 0 if
none matches. We compare GUIDs. When more than one definition has the
record's GUID, the reload can link a record it made to a replaced one,
which the World does not keep; that fails loudly.

**Scope 24 field 1** (0x425be2) is 1 if the selected family's members
(+0x12c, 0x4101f2) include the GUID of the record's linked definition,
which is the record's own (0x4572af); else 0. With no family selected it
is 0. The original reads the selected family's pointer (game +0x10).
Every setter of global 9 to a family's ID stores that family's pointer
with it (0x4f2218 with 0x4f222b, 0x4282b7 with 0x428317), and scripts
cannot write global 9. So a global 9 other than 0 names the selected
family. A global 9 of 0 does not:

- 0x415fc6 and 0x4164f6 clear it and select family 0 (0x415fd7,
  0x416509);
- 0x4278f6 clears it and leaves the pointer;
- the game's start and end set the pointer alone (0x415bd2, 0x415d21,
  0x415ecd).

The World keeps no pointer, so with global 9 at 0 the field fails loudly
(Sol's review found that reading it as no family was wrong when family 0
is selected). A global 9 that names no family is `Trap::Invalid`. A record of a GUID with no definition fails, as the
original would read through a null pointer.

**Verified:** the Goth house is byte-identical through tick 2881, hash
included (`eed688a0...`). The rest is checked against the code only: no
capture lists a menu.
