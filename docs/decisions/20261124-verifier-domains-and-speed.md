# Verifier parameter domains, wider bisimulation, and speed

*Decided 2026-11-24.*

- **Scope:** `crates/sims-antics/src/verify/**`, the `verify-equiv` flags in
  `bin/sims-antics.rs`, and `compile_pack_dir` in `compile.rs`
- **Builds on:**
  - [`20261123-verifier-behavioral-mode.md`](20261123-verifier-behavioral-mode.md)
  - [`20261123-live-output-equivalence-verifier.md`](20261123-live-output-equivalence-verifier.md)

## Context

`verify-equiv` quantifies every tree parameter over all of `i16`. A typed `.antics` v2
tree such as `tree serve(sim: Sim, meal: Meal)` only ever receives one of `Meal`'s
variants, but a legacy tree may still guard the rest with a defensive fallback
(`if param[0] >= 4 { breakpoint; ... }`). A rewrite that drops that unreachable fallback
gets a counterexample at `param[0] = 4`. The difference is real for `i16` inputs and
irrelevant for the program's callers.

## Decision 1: parameter domains, opt-in and always printed

Two ways to assume a tree's parameters stay in a set. Neither is ever on by default;
without them `verify-equiv` is unconditional, as before.

- **`--param-domain <tree>:<index>=<set>`** (repeatable) assumes `param[index]` of tree
  `<tree>` holds a value in `<set>` on entry. A set is a comma list of values and
  inclusive ranges: `0..=3`, `1,4,9`, `-1,2..=5`. It applies in strict and behavioral
  modes.
- **`--v2-enum-domains <file.antics>`** reads a v2 source with the v2 loader. Every
  standalone `tree` parameter typed with an `enum` gets that enum's exact variant values
  as its domain. The parameter is `param[i]`, where `i` counts positional parameters only:
  a leading `sim: Sim` binding and `@ temp[N]` parameters are not BHAV parameters. Trees
  are matched by their BHAV label in the lowered object.

`verify-equiv` prints every assumption before any verdict, one line per parameter, with
where it came from. An assumption the checker cannot use is printed as not applied, with
why: an unknown tree, a parameter the tree does not have, or a kind-4 tree, whose first
parameter is its entry node.

**What a proof then means.** `EQUIVALENT` holds for every run in which each constrained
tree is entered with every constrained parameter inside its domain. It says nothing about
other runs.

**How it is checked.** The live checker splits a constrained tree's entry into regions:
one per combination of ranges of its constrained parameters (at most 256; past that each
parameter's domain is widened to its hull, which only assumes less). A region's entry
path condition bounds each parameter to its range. The regions cover the domain exactly,
so equivalence on every region is equivalence on the domain.

- **Counterexamples stay real.** A counterexample is reported only when `sims-vm`
  replays both trees on a witness and they differ. Witnesses are built from the path
  condition, which includes the region's bounds, and any witness outside the domain is
  dropped before replay. A bug on a value inside the domain is therefore still a
  counterexample.
- **The lockstep checker** proves equivalence for all inputs, so its proofs stand under
  any domain. A lockstep counterexample is never reported under a domain; the live
  checker decides.
- **The bisimulation** starts from the constrained entry. It drops path conditions at
  cuts, so a domain helps only up to the first event. That loses precision, never
  soundness.

**Callers in the same file.** A whole-file run checks the assumption where it can:

- a call with literal arguments to a constrained tree is **discharged** when every
  constrained argument is inside the domain, and **violates** it otherwise. A violation
  makes the callee's verdict, and every verdict that depends on it, inconclusive;
- a call that passes temps (`call(temps)`), or whose arguments spill past the node, is
  **not discharged**. It is listed by tree and node, and the assumption stands there as a
  stated boundary, like any external caller's.

The checker does not track values into another tree's temps, so it does not prove those
calls respect the domain.

## Decision 2: wider cut-point bisimulation

The reversed-layout sweep (every install tree against itself laid out in reverse, under
`--behavioral`) left 5,298 of 23,974 trees inconclusive. Grouping the reasons showed
the largest share was the sweep's own setup, not the checker:

| Inconclusive reason | Trees |
| --- | ---: |
| a call into the global library whose argument count the sweep never supplied | ~4,950 |
| more than 250,000 path pairs to check | 145 |
| a loop that records no event, so neither cut placement applies | 48 |
| a call into a semi-global library the object does not load | ~110 |
| other (Global 34, product-state and path caps) | ~45 |

Changes:

- **The sweep supplies library arities**, as `verify-equiv --global` and
  `--semi-global` do. A call with spilled arguments is otherwise undecidable on purpose.
- **Pairs that take one branch both ways are not counted.** Path and segment-end pairs
  are indexed by the literals of their conditions. A pair whose conditions hold some
  branch atom with opposite outcomes is infeasible: it is the first test
  `feasibility` already makes. Those pairs are skipped without spending the
  pair budget. Nothing else changes, so no new pair is ever accepted.
- **A third cut placement: loop heads.** When cutting at primitives and at every event
  both fail, the bisimulation also cuts each side on reaching a loop head of its own
  tree. A loop head is the target of a back edge in a depth-first walk from the entry.
  The walk follows the graph, so a renumbered tree has the same heads. Cut states pair
  only with cut states of the same event history. The product abstraction is the same
  one event cuts use: literals and equalities, path condition dropped. The induction
  over cuts does not depend on where the cuts are, so the proof stays sound. A
  misaligned cut (one side at a head, the other at an event or exit) fails the proof;
  it is never a counterexample.

Result: with library arities and pair indexing, 23,785 of 23,974 trees prove; loop-head cuts bring it to 23,838 (136 inconclusive, 0 counterexamples). Nearly all of the rest call a tree whose argument count the object does not know. New tests
cover event-free loops: a positive that needs the loop-head cut, and negatives
(another increment, `<=` for `<`, an extra or different temp write, a renumbered wrong
loop) that must never prove.

Multi-exit loops, shared tails and diamonds needed no new rule. The sweep found no
inconclusive tree that failed because of them; diamonds already merge at cuts.

## Decision 3: the last raw operand forms in `.air`

55 of 28,690 install trees still lift with an `op 0x…` line under strict padding.
Grouped by form and by the tree's frame:

- **Out-of-frame operands (49 trees).** The operand names a parameter the tree does not
  take, or a local past its locals:
  - `param[0] == 0` in trees with no arguments;
  - Animate's event local past the locals;
  - Animate's event count read from a parameter of a tree with none;
  - Push Interaction's icon local, Relationship's local, or an expression local past
    the frame.

  These read or write another frame's storage in the VM. `.air` names frame slots by
  the tree's own frame, so it has no honest spelling for them. They stay raw.
- **Remove Object Instance with mode `0xFFFF` (6 lines).** Modes above 2 fault
  (`remove.rs`), so the bytes are a guaranteed fault, not a form to name. They stay raw.
- **Dialog with byte 5 bit 7 set (1 tree): now decoded.** The VM shows `kind & 0x7F`,
  but the dialog record copies the whole byte. `kind` now takes 0 to 255 and lowers
  unmasked, so `dialog(kind: 133, …)` rebuilds the exact bytes. The lowering used to
  mask the bit away, which no source relied on.

## Decision 4: speed

*(Filled in with measurements.)*

## Decision 5: pack GUIDs include catalog variants

`compile_pack_dir` now rejects a GUID that any `OBJD` of one object's IFF defines and
another object also defines, catalog variants included. Before, only the primary and
tile GUIDs were compared, so a v2 `variant` could reuse another object's GUID unnoticed.

## Reopen if

- Domains are wanted on values other than entry parameters (temps, attributes).
- A v2 parameter is passed through temps often enough that undischarged callers hide
  real bugs. That would mean tracking the caller's temps into the call.
