# Verifier round 3: renaming in cut-point proofs, and no default enum domains

*Decided 2026-11-24.*

- **Scope:** `crates/sims-antics/src/verify/**`
- **Builds on:**
  - [`20261124-verifier-round2.md`](20261124-verifier-round2.md)
  - [`20261124-verifier-domains-and-speed.md`](20261124-verifier-domains-and-speed.md)

## Context

The round 3 `.antics` pilot asked for two things:

- **Cut-point renaming.** The MailBox port pins locals with `@slot` in trees 4099 and
  4103, which looked like renaming stopping at the 2048-step path budget.
- **Closed-world enum domains.** Fridges 4105 matches its `meal` parameter over the
  enum `Meal` with no `_` arm, and is proved only with `--v2-enum-domains`. The ask was
  to discharge the default branch in the default `--behavioral` mode when the file's
  own callers pass only in-domain values, staying INCONCLUSIVE wherever a caller
  outside the file could pass something else.

## Decision 1: renaming already applies to cut-point proofs; tests pin it

`check_bhav_live_equivalent` tries each renaming candidate with the same
`check_as_written` the unrenamed pair gets. That function falls back from path
enumeration to the cut-point bisimulation (primitive cuts, then every event, then loop
heads). So a renamed pair is proved by the bisimulation whenever the pair as written
would be. No code change was needed.

The MailBox experiment confirms this. With every `@slot` removed from 4099 and the
`@slot(2)`/`@slot(3)` removed from 4103, the behavioral run is EQUIVALENT on all 95
resources (61 identical, 34 equivalent). One pin stays: `@slot(0) mailbox` in 4103.
Relationship (26) in direction 3 names its local by number in its operand bytes, and the
port's `relationship(dir: 3)` lowers to local 0 whatever slot the `let` got. That is a
compiler matter (the builtin should take the slot of the `let`), not a verifier gap.

`tests/equiv_round3.rs` pins the behavior with a loop path enumeration cannot finish (a
counter compared with an unbounded parameter):

- the loop counter moved to any other slot is proved, and only behaviorally;
- a loop reading a slot it never writes is not;
- a loop reading a stale copy, equal on the first iteration only, is not;
- Dialog text naming the slot (`$Local:0`, or `$Me`) pins it inside the loop.

## Decision 2: no enum domain without an assumption

The default mode never assumes a parameter domain, even when every caller in the file
passes an in-domain literal. A tree's callers are not all in its file:

- **Run Tree By Name (28)** takes the tree's name from a `STR#` 303 of the calling
  code's file (or the global file) and looks it up by BHAV label in the stack object's
  file, its semi-global and the global (`Program::named_tree`). In modes 0 and 2 the
  arguments are the running object's temps, and in mode 1 the stack object's own
  temps (`named_tree.rs`). So code in any other file may run any labeled tree of this
  object, private ones included, with whatever its temps hold. That alone makes an
  intra-file discharge unsound.
- **Interactions** start with zeroed arguments from the pie menu (`orders.rs`), but
  the Action String path sets `args[0]` to the stack object's id
  (`action_string.rs`), and a join sets `args[2]` to the joining Sim. So an
  interaction tree's parameters are not limited to what the file's own calls pass
  either.

Fridges 4105 shows the difference is real. Its only caller in the file is 4106 (`have
a meal or snack`), and no tree of the file calls 4106. It is entered by an interaction
or by name. With `meal` outside `0..=3` the original reaches a Breakpoint (15) and the
port returns true. A closed-world derivation would make 4106's parameters unknown, so
4105 would stay unproved even under a `--closed-world` assumption. No new flag is
added: it would not prove the motivating tree and would only add an assumption to audit.

`--v2-enum-domains` stays the way to prove such trees. It prints an ASSUME line, and
its report lists the file's call sites that discharge, violate or cannot discharge the
domain. The tests pin both sides: the exhaustive match with only in-domain literal
callers is not EQUIVALENT in either mode, and is EQUIVALENT only under an assumed domain
that excludes every differing value.

## Consequences

- Pilot ports may drop `@slot` pins kept for loops beyond the path budget. They keep
  pins a primitive's operand bytes need (`relationship(dir: 3)`), until the compiler
  lowers those through the `let`'s slot (Track 8).
- Trees matching an enum parameter with no `_` arm stay INCONCLUSIVE by default. A
  port that wants a default-mode proof keeps the original's fall-through behavior (for
  example a `_` arm with the original's fault), or the run uses `--v2-enum-domains`.
