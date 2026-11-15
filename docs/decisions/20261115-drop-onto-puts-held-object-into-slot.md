# Drop Onto puts what I hold into a slot of the stack object

*Decided 2026-11-15.*

Drop Onto (opcode 43, 0x44e96a) was not ported. The Pleasants reach it
at tick 934 (person 241, tree 0x2018 node 15) once their load is past
the animator restore and the saved route contexts (open branches), so
it is ported now:

- **Operands.** Two pairs of words: my slot, then the stack object's.
  The first word of a pair says how to read the second: 0 as a number,
  anything else as one of my frame's parameters (native error 8 for one
  the frame lacks).
- **Order.** Both numbers are read first. Then my slot must be one of my
  container slots (native error 0x1b), and the stack object must exist
  (native error 0x15). An empty slot then takes the false branch.
- **The move.** The held object's place test (virtual function 0x54)
  and put (0x58, with the lot told) use the stack object's slot, as Grab
  does. The original gives both the point (-16, -16) and level 1. With
  a container, the place test's record takes the container's position
  but keeps that level (0x432679), so a container upstairs is tested on
  level 1. The put takes the container's level (0x433391).

An ID in my slot that names no object fails loudly, since the original
calls through a null pointer there. No capture covers a Drop Onto yet.
