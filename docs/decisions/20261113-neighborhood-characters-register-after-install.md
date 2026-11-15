# Neighborhood characters register after the install

*Decided 2026-11-13.*

Superseded by "The catalog follows the original's scan order and file
tables" above: characters register after the user objects.

A house's Sims are defined by files in the neighborhood's `Characters`
directory. `Index::add_directory` appends that directory as one more mount,
after Downloads, so the catalog registers characters last. Where the
original registers them is unverified. A family with house number 0 has no
house: in the shipped and installed neighborhoods every such family lacks
flag bit 0, and every family with it has a nonzero house.
