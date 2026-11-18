# Make New Character: the types it makes live in the world, and their GUID takes the tick for the clock

*Decided 2026-11-16.*

Make New Character (primitive 19, 0x44ae08) makes a new person type
while the game runs: 0x4260e1 copies a template's file to
`Characters/User%05d.iff`, gives the copy's definition a new GUID,
registers it and makes a neighbour record for it. The port loads its
program and catalog at boot and borrows them read-only. Two decisions
cover this. Both were made by the coordinator.

## 1. A table of made types at the end of `World`

- `World::characters` (`sims_sim::characters`), the last field of
  `World`, holds each made type by its new GUID: the template's GUID,
  the file number, and the body strings Make New Character wrote. It
  also holds the neighborhood's file counter (+0x160). Saves keep it,
  and the snapshot format goes to **30**.
- A made type is its template's definition looked up under the new
  GUID. Every definition lookup in sims-vm goes through
  `interpreter::object_definition`, which resolves a made GUID to its
  template. Scope 21 (the definition's words) also patches words 14 and
  15 (the GUID, OBJD byte 0x1c) and 70 and 71 (the GUID it was copied
  from, byte 0x8c, written by 0x4725d5).
- A person's profile (body strings 12, 13 and 14) and skeleton (body
  string 0, looked up by name) take the strings the table holds over
  the template's (`people::profile`, `people::skeleton`).
- A record's friend count counts the made types as types whose file is
  in the Characters folder (0x43c3b2).
- The People and catalog stay read-only. The template's file is the
  made type's for everything else: its trees, its semi-global, its
  other strings.

## 2. The GUID: the tick for the clock, keeping the draw

- The original writes (timeGetTime << 16) XOR one C runtime draw
  (0x4725ff, 0x636c74) for each definition in the copy. The port writes
  **(the low 16 bits of `World::ticks`) << 16 XOR the same draw**, so the
  C runtime stream stays aligned. This is an **unverified** substitute:
  no run can match the original's GUID, as it reads a wall clock.
- The original checks no collision: registering overwrites its GUID map
  (0x43bda9 to 0x43bdb1). The port fails loudly when the new GUID is
  another type's, rather than overwrite a type.
- A template whose file holds more than one definition fails loudly
  (each would get a GUID and a draw). The three templates' files hold
  one each: `People\TemplatePerson.iff` in Objects.far, and
  `TemplateDog.iff` and `TemplateCat.iff` in ExpansionPack5.far.

## What else the clone does

- **The file number.** The copy tries the counter, and again one higher
  while the file exists (-0x2b from 0x4722b8); the counter goes up after
  each try, and back to 0 at 32000, where the clone fails. The port has
  no files: a number is taken when a made type holds it. The original
  also finds the files of types made in earlier games, so this is an
  **unverified** substitute.
- **The record.** The first person definition no record links to gets
  the record (0x43ac3a, 0x425770, 0x4257a5 with no slot asked for). The
  neighborhood's load gives every person definition a record, and the
  port deletes none, so it is the new type's.

## The rest of mode 0 (a person)

- The skin tone's name is string local + 1 (counting from 1) of STR#
  0x2000 in the type's semi-global file; an empty or missing one throws
  native error 0x3b (0x44af58), after the clone, whose type and record
  stay.
- Body strings written: 13, the age ("9" below 18, else "27"); 0, the
  skeleton ("child" or "adult"); 14, the skin tone; 12, the gender
  ("male" or "female").
- The bodies and heads it can wear are counted (0x40d077, 0x40d3db;
  lists 0 and 1 of 0x408bf5, built by 0x409663 and 0x40a364). Each suit
  name matching the list's pattern pairs with each texture whose name
  starts with the suit's matched part and the skin tone (0x4727f1 is a
  prefix match folding A to Z to lowercase):
  - a body: "b", any three characters, "m" or "u" for a male ("f" or
    "u" for a female), "c" for a child or "a", any three: nine in all;
  - a head: "c", any three, the same sex and age letters: six in all,
    leaving out names whose second character is "t" (the costumes,
    0x40a453).

  The suits are the avatar registry's names (every `.cmx` and `.bcf`,
  lowercased, but those starting with "suit-"), and the textures every
  `.bmp` on the texture path (0x40895a). `sims_content::Wardrobe` reads
  them at boot from the person files `sims_content::assets` finds (moved
  there from sims-people, which re-exports it). On the Complete
  Collection its counts are the ones the original's patterns give over
  every name: between 17 and 143 for each sex, age and real tone ("lgt",
  "med", "drk"), and 0 for "unknown", the first of PersonGlobals' tones.
  **Unverified:** a texture two folders hold is counted once (the
  original may list it twice; six names, one or two more in five
  counts), and `.tga` files are left out (sixteen, which no pattern
  matches). The lists' order (a hash table's, a file system's) is not
  kept, as it only picks which body and head.- Two game draws (0x46f457): the head, then the body. Which head and
  body they pick (body strings 2 and 1) is presentation and not kept.
- The stack object takes the new record's ID, and the branch is true.
- Presentation, not kept: the copied file, the strings written to disk,
  the derived strings (0x409315, 0x40a588), and the name (0x6e4ac0, STR#
  0x8e string 5, "No childname").

## Fails loudly

A negative age (0x44b23c), a template byte past 2 (0x44afa6), the pets
(0x44b051), a count of 0 (0x44b203, which deletes the type, its record
and its objects through 0x426fcd), and a clone the original fails
(0x426335).

## Where it is reached

When a baby grows into a child (Baby's tree 0x1010), and by a debug
interaction for pets. No strict run reaches it yet.
