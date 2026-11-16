# Make New Character reads its locals, then stops at the runtime definition

*Decided 2026-11-16.*

Make New Character (primitive 19, 0x44ae08) is ported up to the point
where the original makes a new person definition while the game runs.

- Bytes 0, 1 and 2 index the frame's locals: the skin tone, the age in
  years and the sex. The original reads them in the order b0, b2, b1
  (0x44ae23, 0x44ae49, 0x44ae70); an index past the locals throws native
  error 8 (0x44b3b3, 0x44b39e, 0x44b389). Byte 3 picks the template:
  TemplatePerson, TemplateDog or TemplateCat.
- Next, 0x4260e1 copies the template to `Characters/User%05d.iff`,
  gives the copy a new GUID ((timeGetTime << 16) XOR one C runtime
  draw, 0x4725d5), registers it and makes a neighbour record for it. The
  port has no definitions made at run time: the program and the catalog
  are loaded at boot and borrowed read-only. So the primitive fails
  loudly there (`Trap::Unsupported` naming 0x4260e1).
- Two questions decide the rest. The first is where a runtime
  definition lives: a World-side table of made characters (template GUID,
  new GUID, edited body strings 0, 1, 2, 12, 13 and 14) that saves keep
  and that definition lookups go through. The second is what replaces
  the wall clock in the new GUID while the C runtime draw is still
  taken.
- Reached in the base game when a baby grows into a child (Baby's tree
  0x1010), and by a debug interaction for pets. No strict run reaches it
  yet.
