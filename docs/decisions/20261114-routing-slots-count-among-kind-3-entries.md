# Routing slots count among the kind 3 entries

*Decided 2026-11-14.*

Go To Routing Slot and Snap now name a routing slot by its place among
the SLOT chunk's kind 3 entries, as the object's routing slot table
holds them (0x46e2d1), not by its index in the whole chunk. Every route
compared so far went to an object whose routing entries come first, so
no earlier tick changes; object 174 in tick 992 is the first whose SLOT
starts with a container entry. The per-object slot flags and records
written over the type's (`Routing::slot_flags`, `Routing::slots`) are
keyed the same way, as the table's index; the marker's first slot is
the table's first. Snap's own checks of the place (0x4550d8 to
0x4551e9) are ported from the code and not yet reached by a capture:
**unverified** until tick 992's snap into object 174 runs them.
