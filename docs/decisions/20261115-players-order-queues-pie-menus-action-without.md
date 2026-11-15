# A player's order queues the pie menu's action, without its check

*Decided 2026-11-15.*

`Lot::queue_interaction(person, object, menu)` (`crates/sims-vm/src/
orders.rs`) queues the action the original's pie menu makes for that
entry. The object becomes its interaction target, and the action is
built by 0x41f00e with priority 50 (0x458633), then given flags 4 and 8
as a check that passes leaves them. It is queued unchanged (0x48dd67 →
0x45d20b), as a pick does. The check itself (0x458319 in the menu's mode:
rules on the person and object, and the entry's check tree) belongs to
listing the menu's entries, which comes next; until then an order can
name an entry the menu would not offer. A failed order changes nothing,
since the queue can fail partway (it numbers the action before it finds
it cannot place it). Push Interaction (primitive 13, 0x44cd2b) is ported
from the original's code; reading one word past a frame's parameters or
locals, which its bound allows, fails loudly.
