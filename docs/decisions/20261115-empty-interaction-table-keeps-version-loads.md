# An empty interaction table keeps its version and loads

*Decided 2026-11-15.*

A TTAB chunk with no entries was read as version 0, which the
interpreter refuses as a table older than 7. The original's reader
(0x475cc0) reads the version whatever the count, and the loader
(0x475a24) succeeds with no entries. So an empty table now has its own
version, and loads as a table with no entries. A chunk that ends before
the version word, which the original reads past its end, is rejected;
the install has none. In the Complete Collection install there are 270 such chunks:
225 of version 9, 44 of version 8, and one of version 6
(PoolLadder.iff), which the refusal of tables older than 7 still
catches. On the Goth house this moves 32 objects' pie menus past the
refusal; most now reach the tile fallback for an object that offers
nothing (0x48d423).
