# Sound scripts ask about an object as the original answers

*Decided 2026-11-15.*

`getsrcdatafield` passes the values of both its source registers, the
object's ID and a field, to the game's callback (0x57f2e8 calls 0x571127).
We had passed the thread's own object as well and answered 0; the
`Sources` trait now takes the two values and may refuse. A field of 21 or
more is refused, as the original refuses it: the destination keeps its
value and "Illegal source parameter request" is a fault. Any other field
gets the original's object query (0x570e61), listed in
`sims_audio::sources`: field 0 is the voice (1000 for person data word 74
bits 2 or 3, 2 for a child, else the gender word 65), 1 the age, 2, 3,
5 to 7, 11, 12 and 15 to 20 person data words over 10 (skills and
personality), 8 to 10 zero, 14 motive 3 over 10, 4 the hour (global
0: the record at `[0x6e48bc]+0x1c` is the globals block, since its +0x24
is global 10, the lot), 13 the size in tiles of the object's room, or 100.

The original asks the live simulation; the sound here runs on its own
thread. So `sims_game::sound::sources` reads every object's answers from
the world into a `Snapshot` each time the window runs the lot, and sends
it to the mixer before that run's sound events when it changed. A script
can hear a later state than the one its sound was asked in (the window
may run several ticks at once) or an earlier one (the mixer's timing);
nothing in the sound is known to depend on the difference. It only reads the world, so strict output is unchanged.
On the Goth house the answers are what the save holds: Cassandra 2,
Bella 1 and Mortimer 0 for the voice, and their personalities.

**Unverified:** none of the answers has been compared with the running
game, and the meanings of the fields beyond what the code reads (voice,
gender) are guesses from the person data words. Reopen when a capture
of the sound manager's queries can be compared.
