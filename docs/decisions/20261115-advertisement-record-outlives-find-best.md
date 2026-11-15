# The advertisement record outlives Find Best Interaction's checks

*Decided 2026-11-15.*

**Unverified.** The captures do not hold the record's page.

The original keeps one advertisement record. It is the static record at
0x6e5100, and [0x6c437c] always points to it. The frozen memory image
shows the pointer and a count of 16; the advertisements themselves are on
the heap. The check 0x458319 copies an entry's advertisements over the
record (0x4763c5) only when its caller passes somewhere to keep them.
Find Best Interaction does (0x45dc8b); the pie menu (0x45871f) and the
at-once queue (0x45d259) pass 0 and copy nothing.

The order inside 0x458319 is:

1. An entry ruled out by the target's flags, the visitor rules or the
   person's age and kind returns before the copy (0x458497).
2. The copy.
3. The check tree, if the entry has one. With no tree the check passes
   (0x458524).
4. When the target is in another room than the person, the room motive's
   delta becomes its minimum negated, in the record (0x45853f). This
   happens whatever the check returned. Both rooms are read after the
   check tree (0x45852f, 0x458536), so a check that moves either object
   counts. Before, the port compared the rooms from before the scan.

Scopes 12, 28 and 29 (0x453a09, 0x453a42, 0x453a7a) read [0x6c437c] in
any tree, so a check tree the pie menu runs reads and writes whatever the
last Find Best Interaction check left. The record is now a World field,
`advertisement_record`, and the scopes use it everywhere. Their index is
bounded by 16 (native error 0x1a), as before. This replaces the
check-tree-only advertisements and the loud failure outside a check tree.

0x4763c5 copies as many advertisements as both the entry and the record
have, and the record's count stays 16. So an entry with fewer than 16
leaves the rest as the last copy left them. That case used to fail
loudly and is now ported.

Before the first copy, the scopes and a copy of fewer than 16 still
fail loudly: the record then
holds what the game's start left on the heap, which nothing here reads.
On Goth, object 18's pie menu at tick 1 stops there, before autonomy
first runs (tick 232). Reopen when a capture holds the record's page, or
when a recording picks from a menu that reads it before the first copy.

Snapshot format 22 adds the record at the end of the World. Each pin of
the Goth ticks 417 to 1000 now reads through format 21 and holds as it
was.
