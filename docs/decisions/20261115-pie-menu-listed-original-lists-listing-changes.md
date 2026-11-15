# The pie menu is listed as the original lists it, and listing changes the lot

*Decided 2026-11-15.*

`Interpreter::pie_menu` lists what the original's pie menu offers a
person on an object (0x48d392, 0x45859c): every entry of the lead tile's
table in order, each made into an action by its menu number (find-first,
so a repeated number repeats the first entry's action), checked by
`Machine::check_chosen`, and kept when flag 8 is set and 0x10 clear.
Nothing is greyed out; a failed entry is just absent. `Interpreter::pick`
queues a listed action unchanged (0x48dd67).

Listing is not a pure query: the check trees run synchronously on the
person and their effects stay, apart from the temps, which are put back
once at the end; person word 50 is left as the last check set it. The
original lists the menu when the cursor reaches an object (0x48c834), so
a caller wanting parity lists it at the same moments, and a recording of
play must note those moments as inputs. The hover's highlight bits
(object +0x114) are not modeled.

Ported from code alone; **unverified** until goal step 4's recordings.
Taken as clear: the interface flag 0x6f2a54, which hides every entry.
Fails loudly: a pet's menu (it reads interface state 0x6e4f68), a check
tree that adds its own entry (opcode 50 in the menu build, 0x458789),
the entry to join another person's joinable interaction (0x45866d), and
an object that offers nothing, where the original lists the objects on
its tile (0x48d423).
