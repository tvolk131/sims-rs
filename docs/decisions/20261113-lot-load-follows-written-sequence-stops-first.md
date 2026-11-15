# The lot load follows a written sequence and stops at the first gap

*Decided 2026-11-13.*

The load sequence in `docs/roadmap.md` lists the ten steps the original runs between
reading a house file and the first tick, with what `sims boot` does for
each. `sims boot` runs them in order and stops at the first step that is
not implemented, so the hash it prints is always of a state the original
passes through. The alternative, running whatever callbacks already work,
would give a state the original is never in, and its hash would mean
nothing to the comparison harness.

The first step added is the globals the running game supplies
(`World::apply_context`). The values for a Complete Collection install
(mode 0, global 25 zero, free will on, expansion mask 0xff, performance
class 2) are the old prototype's headless profile and are **unverified**.
The performance class comes from the machine's memory and CPU speed, so
the comparison harness must read all five from the running original.

Writing the sequence down showed that the catalog order was wrong for load
step 4, where new objects get IDs in catalog order. [20261113-catalog-follows-originals-scan-order-file-tables](20261113-catalog-follows-originals-scan-order-file-tables.md) fixes
it.
