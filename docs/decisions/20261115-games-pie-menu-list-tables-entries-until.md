# The game's pie menu is a list of the table's entries until the simulation offers them

*Decided 2026-11-15.*

Superseded by "The game lists the pie menu on the lot when the pointer
reaches an object" above, except where the simulation cannot list a
menu.

Clicking an object (or a person) in `sims-game` lists the entries of
the interaction table `Lot::queue_interaction` queues from: that of the
tile the simulation sends the order to (a multi-tile object's tile whose
definition's word 8 is positive sends it to the tile whose word 8 is
that number negated, as `interaction_target`, 0x435286, does), read with
`Program::interactions`. Each menu number is listed once, in table
order, named by the TTAs string table with the same ID as that TTAB, in
the file that holds it. The game repeats the simulation's choice of
tile and of TTAB ID from the program's public data, because the
simulation keeps both private; it names the entries only when the table
it finds is the very one `Program::interactions` returns.

The original offers only the entries whose check passes for the selected
Sim; that check is not run, so the list can hold entries the original
would not offer, and choosing one queues it anyway. Entries with flag
0x80 are hidden: every Debug and Force entry of the Goths' house has it
and nothing a player is offered there does (**unverified** as the
original's debug flag). The menu is a plain list at the pointer, not the
original's pie: in columns when taller than the view, and as many
columns as fit, the last row then saying how many entries are not shown.
A dialog closes it. Reopen when the simulation can list the entries it
would offer, or `sims-ui` draws the pie.
