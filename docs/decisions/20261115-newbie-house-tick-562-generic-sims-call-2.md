# The Newbie house to tick 562: Generic Sims Call 2, a catalog picture headline; Breakpoint and Show String

*Decided 2026-11-15.*

**Unverified.** No capture of the Newbie house (`House07.iff`) exists
yet; each piece is ported from the original's code and checked by
synthetic tests. The Goth house's strict output is unchanged line
for line through tick 2881, hash eed688a0… included.

The base neighborhood has two families, the Goths (House05) and the
Newbies (House07); its other houses hold none. In strict mode the
Newbie house stopped at tick 245; it now runs to tick 562.

**Generic Sims Call routine 2** (0x44fb51 to 0x44fbd8). The stack
object must be on the lot (the lot's object table, +0x18 of the
running object's lot +0xdc; native error 0xa otherwise), checked before
the running object is cast to a person (error 0x1c). If the person has
a running action (+0xa84 not 0, the test the route's end uses), the
stack object becomes that action's icon (+0xa54). The branch is true.

**Set Balloon/Headline kind 7** (0x44b811 to 0x44b8d8): a catalog
picture, picked by byte 2 as it is, not with the temp option bit 12
adds (the code keeps both at 0x44b7d6 and adds the temp to one):

- 0: the stack object's;
- 2: the object a local names, the local being operand word 0 shifted
  right by one, signed; not below the frame's local count (frame byte
  6) is native error 0x33, and a negative local fails loudly, as the
  original would read before the locals; so does a local that still
  holds half of an old pointer in the frame storage, which the original
  reads as it is;
- 1: a neighbor's (0x4256f4, the neighborhood record's +8), which is
  **not ported** and fails loudly: whether a record holds its picture
  is presentation we do not model;
- any other byte ends the call.

An ID that names nothing on the lot (not positive, or a null entry of
the lot's table) ends the call too, after the priority is written, as
for an icon with no sprite. The picture is the definition's (+0x11c),
loaded into its cache (+0x8c, 0x45728d) when first asked for, which is
presentation. The record's frame count becomes 1 (0x46dfe9), so option
bit 11 makes the duration 12 ticks a loop. The rest is the other kinds'
path: backdrop, overlay, flag, duration, redraw, wake.

**A head looking at an object that is not a person** (0x59058f to
0x5905d0) was ported here and, at the same time, by #156 for the Goth
house's tick 2837; #156's port is kept. The two disagree on the order
of the coordinates, and **the disassembly reads as the opposite of
#156's**: the caller copies the object's +0xf4, then +0xf8, into the
point it hands 0x454f5c, which stores 3/16 of the first word (`fild
[esi]`) as the target's first coordinate and 3/16 of the second (`fild
[esi+4]`) as its third. +0xf4 is our second coordinate (`animator::place`,
`direction.rs`, `route.rs`), so the target would be `[3/16 y, 0, 3/16
x]`, as for a person's model matrix (`head::placement`), where #156
writes `[3/16 x, 0, 3/16 y]`. The swap changes the Goth house's hash at
tick 2881 (eed688a0… becomes cdd5b72c…; every tick's line is the same,
so only the animator's head state differs so far). It is left as #156
has it, and the strict output is unchanged; **reopen** with track 3,
and with a capture past tick 2837, which would settle it.

**Breakpoint and Show String** (opcodes 15 and 21), which `sims
census` counts 1,489 and 27 times in the install's trees, are ported
whole from the general table (0x45246a); no house has reached either
yet. Breakpoint (0x451e34) reads the word operand word 1 (scope) and
word 0 (index) name, with the reader every primitive uses (0x453141;
a scope above 38, or a negative one, is native error 9), and a word
that is not 0 is native error 0x3e. Show String (0x451ab7) formats a
string from the running tree's file and sends it as signal 0xeb, which
none of the game's eight subscribers takes (each handler's cases were
read: 0x41637f, 0x43c0e4, 0x44302e, 0x46d2c2, 0x47b22a, 0x4a82db,
0x4c3ea5, 0x4f65bf), and its string lookups fail on nothing; so it
changes nothing kept, and its branch is true.

**What stops the Newbie house now.** At tick 563 person 137's head is
measured while its base clip (`a2o-standing-loop`) moves a bone its
foreground clip (`a2o-disgusted2`) leaves alone: the skin's blend of
the two layers (0x5992b3) is not ported.
