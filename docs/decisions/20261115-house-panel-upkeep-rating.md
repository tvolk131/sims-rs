# The house panel's Upkeep rating

*Decided 2026-11-15.*

The house panel's Upkeep bar is now worked out, read from the exe. It was drawn empty before
(`20261115-house-panel-size-yard-and-facts.md`). `sims_ui::house`
works it out as `House::read` does the other ratings.

- **The scan** (0x4452eb) goes through every object on the lot (the
  lot's list at +0x30, linked through +0xe0). Here that is
  `World::objects`; the sums do not depend on the order.
  - A broken object (data word 15, object +0x68, not 0) adds the
    broken penalty. Any other object that is dirty (data word 39, +0x98,
    above 0) adds the dirty penalty. This is checked for every object,
    counted or not.
  - An object is counted if its type record's +0x4e, +0x50 or +0x8a is
    not 0.
  - The same loop also adds up object values for something else
    (0x43219a, into the stats' +0x1c and +0x20). That is not ported
    here.
- **The type record holds the definition's bytes from its start**, the
  version included. The person type at definition +0x12 and the flags
  word at +0xcc (word 102) already read so in `sims-vm`. So +0x4e,
  +0x50 and +0x8a are OBJD words 37, 38 and 67 after the version: the
  room flags, the function flags and the build mode type. This corrects
  the earlier guess of words 38, 39 and 68.
  `house::Kinds` reads them for each GUID on the lot from the install's
  definitions, as the original holds them (0x431759, as `sims-vm`
  reads them): a version 136 or 137 definition shorter than 216 bytes
  and of odd length loses its last byte, then every definition is
  filled out with zeros to 216 bytes. A word cut in half keeps its low
  byte.
- **The rating** (0x4168d4):
  - It is the count less the penalties, over the count. The ratio is
    kept to 0 to 1, with an unordered value taken as 0, then multiplied
    by 100 and cut toward zero.
  - The arithmetic is single precision. The sums are 32-bit and wrap.
  - With nothing counted the rating is 0.
- **The penalties** come from the global file's tuning table 4, by name
  ignoring case: "upkeep penalty per dirty object" and "upkeep penalty
  per broken object". They are cut toward zero when read (0x42434a,
  0x42436c). Where the table lacks a name, the original's values are 6
  and 15 (0x666cbc, 0x666cd0). The install's table has 3 and 6.
- **When the types are read:**
  - The game reads them for types it has not seen yet, at each draw
    while the house panel shows.
  - The viewer's world does not change, so it reads them once when it
    opens.
  - A type that has not been read counts for nothing.
- **The Goths' house:** after the load, 73 of the lot's 253 object
  types count. Nothing is dirty or broken, so Upkeep is 100.
  **Unverified** against the original's screen.

**Tests:**
- The penalties and the count on a hand-made world: a broken object
  that is also dirty, an object that isn't counted but takes a
  penalty, a type that hasn't been read, more penalty than count, and a
  negative penalty.
- Which definition words count, including short definitions, a word
  cut in half, and versions 136 and 137.
- The tuning's penalties: the defaults, and the table's values cut
  toward zero.
- The Goths' panel digest.

**What would reopen it:** a screenshot of the original's house panel
with dirty or broken objects on the lot.
