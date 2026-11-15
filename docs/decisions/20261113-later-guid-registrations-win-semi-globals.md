# Later GUID registrations win; semi-globals resolve to the first match

*Decided 2026-11-13.*

The catalog registers object definitions in load order, and a GUID resolves
to its last registration. The old prototype recovered that rule from the
original's lookup. Multi-tile masters are the exception: the first
registered master wins, and that is for the interpreter to apply. A GLOB
link resolves to the first file of that name in load order. That part is
unverified: the Complete Collection has no duplicate semi-global names, so
nothing tells the two rules apart yet.
