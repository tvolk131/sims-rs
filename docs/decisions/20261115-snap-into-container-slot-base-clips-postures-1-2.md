# Snap into a container slot, the base clips of postures 1 and 2, and K = 1000

*Decided 2026-11-15.*

Snap into an object's container slot is ported, with the place test for
a slot (0x432d62 to 0x432fb8), from the original's code. Only one
container placement is reached so far: person 21 sitting in object 174
in tick 992. A container slot off the object's centre would move the
person's animator offset (0x454ceb), which is not kept, so it fails
loudly. The steps it does not reach (mode 2's centre rule, the
other slots at the same height, the containers around the object) are
ported from the code alone and **unverified**. The base clips of
postures 1 and 2 are looked up and fail loudly when missing or when
they have timed events. Posture 0's clip is still not looked up, since
every capture shows its layer after a start. A clip that is never looked
up cannot fail, so reopen this if a person's posture-0 clip could be
missing. K = 1000 on the Goth house. That is step 1 of the goal; next is
a full game day.
