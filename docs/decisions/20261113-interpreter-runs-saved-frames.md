# The interpreter runs on the saved frames

*Decided 2026-11-13.*

An object's thread is its stack of saved frames in `World`, so the
interpreter keeps no state of its own and every snapshot holds a resumable
stack. Frames name their code owner by objt type ID, so `World` now keeps
the lot's type table (`types`, type ID to GUID; snapshot format 2).
Expression, Random Number and the operand scopes are ported from the old
prototype with its native error codes. Scopes over state `World` lacks
(tree advertisements, definition words, rooms, neighbors, jobs,
neighborhood data, type attributes) and every other primitive fail loudly.
Four semantics are **unverified**:

- New frames start with zeroed locals. The original reuses a frame arena,
  so locals inherit words left by earlier frames; saves carry no arena.
- Random Number draws from a stream the caller names. The original swaps
  the lot stream in only around the lot update (0x46d553).
- Motive reads round half to even (the x87 default); the prototype rounded
  half away from zero.
- Motive arithmetic rounds once from the x87's 64-bit significand to 32
  bits. The FPU precision the original runs it at is not known.
