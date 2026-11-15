# A person wears what their suit picks, a career's work clothes included

*Decided 2026-11-15.*

Mortimer Goth changes for work at 8:07 (the simulation sets his suit,
person data 8, to 3 at tick 2906) and leaves in a white lab coat; we
drew him in his everyday clothes until he came back. sims-people now
follows the suit as the original's outfit choice (0x58a178) does, for
the suits whose clothes come from the person type's body strings or
from matched names (`sims_people::suit`): 1, 2, 4, 17 and 18 from
their entries; 3 from the career level (0x45ee5f); 5 from entry 33 or
matched pajamas (0x45f034); a body left without a name falls back to
entry 2 (0x58d492). Suit 0 keeps the record's own clothes, as before.
A person whose record names no clothes (one a script made) starts from
their type's default outfit, and their suit is applied to that.

A career level names a male and a female body and a texture, as
templates (`Science$g$b_01`, `Science$g$c_01`): `$g` is the sex's
letter or `u`, `$b` the body type from entry 2's name, `$c` the skin
tone (entry 15). The body is the first appearance the template
matches; the texture the best-ranked texture name, with entry 2's
texture ending (`_rat`) as an optional tail, written `BODY=<name>`
(0x40e5ce, 0x40e674, 0x472a25). For Mortimer, science level 0, that is
`sciencemfit_01` with `BODY=sciencemlgt_01`, and the coat matches the
original's screenshot at 8:11 in cut and colour. The template matching
follows an earlier port (`crates/sims-sim/src/pose/clothing.rs` on the
`alpha/playable-households` branch), checked against the disassembly
for the career and pajama paths.

Approximations, each reopening this if a case turns on it: the
appearance and texture names are taken in name order, where the
original keeps its registries' load order (only a tie between two
equally good names depends on it; none of the careers tried has one),
and TGA textures are not listed. Not ported: the costume suits 6 to 60
other than 17 and 18 (the record's clothes are kept), the hands the
hand words (data 22, 23) pick, the profile migrations run when a
profile is bound (0x409315), the career cache that keeps old work
clothes when only age or sex changes, and a `Careers.txt`, which the
original reads in place of `work.iff`.
