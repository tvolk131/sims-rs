# Scope 24 reads neighbor fields; field 9 is the family friend count, cached in the family

*Decided 2026-11-15.*

**Partly verified** (the load's count and the threshold, against the
capture of the Goth house's first tick; the reads during the social pick
are **unverified**).

Scope 24, "neighbor in stack object" (0x453389), hands the stack object
and the field to the neighborhood's 0x425b5d. That finds the record by
its place plus one (0x4256f4), as scope 32 and Set To Next do. No
record gives 0 for every field, where scope 32 throws 0x37. The field
switch (0x425d41) has ten cases; above 9, or negative (compared
unsigned), is 0. The value has no word pointer: with write intent the
caller's pointer is cleared (0x425b8e), so a write is native 0x13 after
the field is read.

- **2** is person data word 58 (+0xe8); **3** and **4** are 0.
- **6** walks the families (neighborhood +0x154) for the first whose
  members (+0x12c, 0x4101f2) hold the record's GUID, and gives its house
  (+0x110), whatever the record's word 61 says.
- **9** finds the first family whose ID (+0x10c) is word 61 (0 gives 0)
  and, with or without a house (0x425cfa does not read +0x110; the house
  checks are fields 7 and 8's), returns 0x410260 → 0x427219: the
  family's cached count (+0x120), worked out again first by 0x427bd7
  when the stale byte (+0x124) is set.
- **0**, **1** and **5** read the record's +8 (a pointer to the person's
  type data: 0x423f30 reads +0xb4 from it, 0x4572af +0x60). **7** and
  **8** read the family's flags (+0x138), whose bit 1 the statistics
  pass rewrites (0x4160f9), which is not ported. All five fail loudly,
  naming their address.

The count (0x427bd7) walks the records in slot order. It skips a record
whose GUID is a member's, or whose word 61 is 0. For each member GUID it
takes the first record of that GUID (0x42571f, none for GUID 0). The
record counts once, at the first member it is friends with both ways
(0x4271c5: each has a row for the other with at least one value, and
both first values are at least the friendship threshold), if its kind
(word 65) is at most 1. The threshold is 0x6c4398: the autonomy tuning's
"friendship threshold" (default 25.0), stored truncated (`__ftol`,
0x459120). The capture holds 50 there, and our tuning reads 50. Then the
stale byte is cleared and the count stored.

The family's constructor (0x41003c) sets the stale byte. The FAMI reader
(0x4104ce) stores the saved count but leaves the byte. It is set again
only by 0x4246b9, which a relationship write calls for both records
(0x44485b): it marks the record (+0x24) and the first family whose ID is
the record's word 61. (Family moves, 0x425f66 and 0x42601e, set it too.
They are reached only from Generic Sims Call routines that are not
ported, and from the interface.) The count is not a function of the
current state: a change to anything else it reads (word 61, word 65, a
GUID, membership) leaves the old count until a relationship write marks
the family.

So the World now keeps the families (`World::families`, appended;
snapshot format 23): ID, house, members, count and stale flag, in the
reload's order. The members are filtered to GUIDs some record has, as
the reload's first step already said. Budget, number, flags and the rest
of the record are not kept, since nothing ported reads them. The
relationship write now marks the family, which the earlier port left
out.

Load step 20 is the interface's family panel (0x4cfcdb) showing the
count, so the load works out the selected family's (global 9) count.
In the capture at the first tick the Goth family (5) has count 0 and is
up to date, and the other 25 families have the file's 0 and are out of
date. Our state matches, and `resize.rs` pins it. For an empty lot
nothing is counted (unverified).

**The panel's redraw.** The panel shows the count again whenever the
game updates its windows. The redraw is slot +0x17c of the
`cWinViewControl` vtable at 0x66f220 (stored at 0x66f39c), 0x53a01f,
which the generic window update 0x5dce19 calls at 0x5dd0de. It builds
the label through 0x4cfcdb → 0x410260 → 0x427219, which works the count
out again if it is out of date. The game updates its windows as it
handles its messages: between lot updates, and inside the pump that
waits for a dialog that stops the game (0x436da0 → 0x4f6b95 →
0x5dab4b). So `Interpreter::refresh_family_panel` counts the selected
family again after each of `Lot`'s lot updates and as the registry
shows each such dialog.

This matters for more than the stale flag. A count is cached until a
relationship write marks the family, and a relationship read makes
missing rows (zero values) without marking it. With a threshold of 0, a
write, then a redraw that counts 0, then a read that makes rows, then a
read of field 9 gives the cached 0 in the original, where a family left
out of date would count 1. A test covers the cache across a redraw.

**Partly verified.** In all 608 captured updates of the Goth house,
family 5 is up to date after the update; only update 2 changes
relationships (record 32's row for 34, from [50] to [50, 0, 50], which
marks family 5), and update 2 shows a dialog. The snapshots do not show
which redraw cleared the mark. Whether the window update redraws the
panel on every pass, or only when the panel is marked, is
**unverified**. Redrawing more or less often differs only when a
relationship write and a read of field 9 fall between the same two
redraws while something else the count reads changes. The tick pins
leave the families out, so they are unaffected.

Strict mode: through tick 1102 every line of `sims boot` matches main
except the final hash. That hash covers the new families field; without
it the state equals main's. The tick-33 pin shows the same: its format
22 digest is the old pin. The K=1000 tick pins in `resize.rs` hold
unchanged.

**On Goth.** The social pick reads field 9 at tick 261 for record 34
(word 61 = 5, the Goths). The social's relationship writes to records
32 to 34 had marked family 5 again, so it counts again: 0. The social
then stops in the same tick on object 21, tree 0x200e node 21: Set
Balloon/Headline with an icon of kind 6 (0x44b7ed), which is not
ported.

The old prototype's `family_friends.rs` agreed on the count, the cache
and the marks. It also kept the family flags and a per-family dirty
plan. Reopen if a capture of a relationship write shows the family's
stale byte untouched, or a capture shows a redraw at a point other than
between lot updates and a dialog's wait.
