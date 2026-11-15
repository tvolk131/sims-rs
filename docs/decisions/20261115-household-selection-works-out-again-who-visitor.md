# The household selection works out again who is a visitor

*Decided 2026-11-15.*

The Pleasants' house, as the original saved it after moving them in
(`House06.iff`), holds the Traveling Salesman, an NPC of no family whose
visitor word (person word 32) is 2. The end of the household selection
refused any person not of the family, so the house did not load. It now
does what the original does (0x428041 from 0x4283cf;
`World::finish_selection`):

1. A person whose type is a neighborhood character, of no family, whose
   word 67 is not this house, is deleted. A type is a neighborhood
   character when it was registered from the neighborhood's `Characters`
   folder: 0x43c228 registers that folder with kind 1, which 0x43ba52
   keeps in bits 2 and 3 of the type record's +0xb4, and the scan tests
   for 4 there. Here those are the types whose resource comes from the
   `Characters` mount (`Index::CHARACTERS`).
2. Global 26 and flag 4 of the family's record still fail, as before.
3. Each person's word 32 is worked out again (0x42a2b4): a 0 becomes 1,
   then 0 if the person's type is one of the family's members (0x41019b,
   the family's member list at +0x12c), then on a lot not zoned
   residential (0x40745f: the zoning of global 10's house, house 0
   residential) a 0 becomes 1 again. Global 10 is read as it stands at
   this step, not the house number the load was given: an object's Load
   or Init tree may have changed it, so `World::finish_selection` takes
   the neighborhood's zoning and reads the global itself.
4. A neighborhood character who was a resident before the selection (word
   32 was 0 in the note taken at 0x42821a, `World::residents`) and is now
   a visitor is deleted (0x4285e2). The family number check after it is
   for a number below 3000 and from 3010 on, which no number is, so it
   deletes no one.

The deletions are not ported and fail before anything is written. The
salesman is not a neighborhood character (his type is from
`ExpansionPack7.far`), so he stays, with word 32 still 2, as the capture
has him.

Reading the people also raises the action serial (global 0x6e4904): the
interaction reader (0x41f1c5) ends with the serial taking the
interaction's number when that is larger, compared signed (0x41f320),
and it reads each person's running, last and queued interactions
(0x45d012, 0x45d01f, 0x45d08b). Nothing resets the serial. The Goths' and
the Newbies' numbers are all 0, so this changes nothing there; the
Pleasants' largest is 4, as the capture's serial is after the load.

With these, the Pleasants' load matches track 2's capture
(`house06-pleasants-ticks-20261115`, entry) in the streams, the globals,
the action serial, the object IDs and every object's data, temps,
attributes, frames and person data, but for two things:

- Person 241's fourth frame has tree 0xa029 in the capture and 0x2029
  here and in the file, like object 19's 0x8118 in the Goth captures.
  The frame reader (0x474694) writes only the low 15 bits of the tree
  word and keeps bit 15 from the frame arena, which is freshly allocated
  and not cleared (0x4750ed clears only its first four bytes); every
  reader masks the bit off (0x4746c0, 0x4747ea, 0x474f79). The bit is
  left over from the heap and nothing reads it, so it is not kept.
- Person 243, the Sim selected, has object data word 50 at 1 and its
  last result true in the capture, 0 and false here. Word 50 is the
  word a check tree sets; something in the original's load runs a tree
  on 243 that is not ported, and what is not found yet.

The first frame then stops. Person 242's head follows person 241 (word 41
is 241, word 42 is 1), and 242 is drawn before 241, whose animator has
never been ticked or drawn. The original measures the point on top of
241's head from the anchor's bone matrices as the skin's construction at
load left them (0x59293d, 0x59640d, 0x59638f); measuring also places the
animator (0x592132 sets each unset coordinate from the person's). What
those matrices hold is not in any capture, so it is not ported and the
frame fails naming it. In the capture 242's word 42 stays 1 until its
countdown ends at update 36, so the turn never reached its limit.

The strict Goth run is unchanged to tick 3602, as is the Newbie run.
