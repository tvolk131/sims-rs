# Some installed houses do not load yet, on purpose

*Decided 2026-11-13.*

`sims houses` loads 660 of the 718 house files in the install's eight
neighborhoods and the shipped one. The 58 failures:

- 18 are `House00` and `House11` in each neighborhood: base-game templates
  with ObjM versions 0x33 and 0x38, whose record layout is not known yet.
- 40 are houses 56, 63, 69, 76 and 77, which hold Sims whose character files
  are not in the neighborhood: Superstar's community templates, and houses
  the old harness saved. The original presumably repairs or drops such residents,
  which we have not reverse engineered.

Neither blocks the target house. Reopen when a goal needs those houses.
