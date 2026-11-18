# The personality panel shows the sign of the zodiac

*Decided 2026-11-15.*

The personality panel now shows the selected Sim's sign of the zodiac
under the title (`household::zodiac`, `Resident::zodiac`, drawn by
`hud`'s `draw_zodiac`), read from the exe.

- **Which sign** (0x51cd11): person data 70 holds it, 1 (Aries) to 12
  (Pisces).
  - Where it is 0, the panel works it out (0x4841fc). Person data 6, 2,
    5, 7 and 3 (Outgoing, Nice, Playful, Neat, Active), each times the
    float 0.01 and kept as a float, are measured against the 12 signs'
    traits (0x6c5bf8). The sign with the least sum of squared
    differences wins, the first on a tie. The game sets the x87 to
    24-bit precision (0x4fad5c clears the precision bits), so every
    step rounds to a float, and `f32` arithmetic gives it exactly. With
    fractional traits this decides ties that wider arithmetic would not
    (a test pins one, from Sol's review).
  - A saved sign is held to 1 to 12, as the name lookup (0x4842de)
    holds it.
  - A person of a kind above 1 (person data 65) is shown no sign.
- **Not ported: the write.** The original saves a sign it works out
  into the person's data word 70 (0x51cd44), so opening the panel
  changes the simulation's data. The UI here only reads. The Goths
  have their signs saved (Mortimer Taurus, Bella and Cassandra Cancer),
  so nothing differs for them. A person with none saved would have
  word 70 still 0 here after the panel shows their sign; a script that
  reads it would see the difference. That write belongs to the host
  or the simulation, if it is wanted.
- **Where:** the name in font 12 (0x6f2af4), sized to its text, at 5
  from the subpanel's left and the line plus 5 above its bottom
  (0x51cdc4..0x51cdfe). The names are UIText.iff STR# 164, strings 1
  to 12 (0x48410b).
- **Unverified:** the color (the labels' is taken).
- **The help balloon** over the sign (0x51bfb3), which names the most
  and least compatible signs, is drawn since 2026-11-16
  (20261116-sign-help-balloon.md).
- **Tests:**
  - each sign's own traits give it, and a tie keeps the first;
  - saved, clamped, and hidden signs;
  - the drawing's place;
  - the Goths' saved signs agree with the ones their traits give, and
    the panel's digest is pinned again.
