# Verifier upgrades from the multi-asset `.antics` pilot

*Decided 2026-11-24.*

- **Scope:** `crates/sims-antics/src/verify/**` and the `verify-equiv` flags in
  `bin/sims-antics.rs`
- **Builds on:**
  - [`20261124-verifier-domains-and-speed.md`](20261124-verifier-domains-and-speed.md)
  - [`20261123-verifier-behavioral-mode.md`](20261123-verifier-behavioral-mode.md)
  - [`20261123-live-output-equivalence-verifier.md`](20261123-live-output-equivalence-verifier.md)

## Context

The multi-asset pilot ported eight objects to `.antics` v2 and proved them against the
originals with `verify-equiv`. Its reports list verifier findings:

- **V3 / VR-1:** a wrong `route_to` source (1 instead of 2) came back INCONCLUSIVE.
  Primitive operand differences are only ever "possible", and the loop around the
  primitive could not be closed.
- **V1:** behavioral mode compares local slot numbers literally (Animate's event local,
  `let`s numbered in another order).
- **V4:** the frame-shape check refused 0 → 1 locals but accepted 2 → 14.
- **V5:** a path that faults compares its last branch result.
- **VR-3:** one unproven tree makes every caller INCONCLUSIVE, so the whole-file verdict
  says nothing.
- **VR-2:** unbounded loops cap verification.
- **Track 8:** Set To Next's explicit `dest: stack_obj` differs from the default form.

Each decision below keeps the verdicts' contract: EQUIVALENT is sound, DIFFERENT is a
run that differs, everything else is INCONCLUSIVE.

## Decision 1: decisive operand differences on the first primitive

When a comparison cannot decide (a loop it cannot close, or a difference after a
primitive), the checker runs both trees once more, ending every path at its first
primitive or call. It reports DIFFERENT for two paths that meet all of these:

- nothing before that event is an event at all: no primitive or call (it might never
  return the way the path needs), no opaque write, no opaque read. So the running
  object, the stack object and its definition and position are whatever the entry had.
  An earlier event could rule out every world that tells the operands apart:
  - after `stack_obj := -1`, both runs fail before they look at a slot;
  - a read of `global[-1]` faults in every world (`memory.rs:103`).

  The paths' conditions also name no event's outcome. A condition that tests the stack
  object has no concrete witness, so it never qualifies either;
- they reach the same primitive with equal event fields and state (tracked cells,
  scratch pointer, last result);
- their joint condition holds for a concrete witness on its literal input bounds, and
  the witness lies in any assumed parameter domain;
- under that witness, their operands select behavior that differs in some world;
- the tree runs its own nodes from a known start. A tree ID with bit 15 set never
  qualifies: the VM looks a frame's tree up without that bit (`saved_objects.rs:29`), so
  0x9000 runs 0x1000. Nor does a kind-4 tree, whose entry picks its first node.

Only Go To Routing Slot (opcode 45) is modeled (`go_to_slot.rs:313`). Both runs first
test the same things: a person runs it, a stack object exists and stands on the lot. Then
each looks up its slot:

- source 0: its own slot `param[number]`;
- source 1: its own slot `number`;
- source 2: global slot `number`;
- any other source: native error 0x25.

A stack object's definition is part of the world, and its slots are an array. So a
definition can be chosen to tell these apart:

- two different own slots, at least one of them ≥ 0;
- an own slot ≥ 0 against a global slot or an invalid source.

The global slot table is fixed game data the checker does not know. So two different
global slots, or a global slot against an invalid source, are not decisive.

sims-vm replay does not run primitives, so this kind of DIFFERENT says "Not replayed in
sims-vm" and gives the witness and the world that tells the runs apart. A call or
primitive before the differing one keeps the result INCONCLUSIVE, because it might never
return as the path needs. The Bookcases case 4107 (`store_original_points(sim)?` before
`route_to`) is therefore still INCONCLUSIVE.

## Decision 2: Set To Next's explicit stack-object destination is the default

Without bit 7 of byte 4, Set To Next ignores bytes 5 and 7 and resolves
`resolve(10, 0, true)`. With bit 7 it resolves `resolve(ops[5], ops[7], true)`
(`set_to_next.rs:119`). So bit 7 with scope 10 and index byte 0 is the same cell
through the same call. Canonical operands now rewrite that spelling to the default.

The canonical form feeds events, the relaxed lift and pads. It does not feed a strict
faulting node, which still compares raw bytes. So the two spellings prove equivalent
under `--behavioral`, and strict mode stays as it was.

## Decision 3: behavioral local renaming

Under `--behavioral`, when the trees are not proven as written, the checker tries
renaming one side's locals to the other's numbering (`verify/rename.rs`). It takes the
renamed result only when that result is EQUIVALENT.

A renaming permutes the *clean* slots of a tree:

- **Clean:** on every path from the entry, something writes the slot before anything
  reads it. The writes that count are `Expression` op 5 or 20 (they write without
  reading, `expression.rs:75`) and Animate's event store. With flag 0x20, Animate writes
  `local[ops[2]]` on every false branch and only there (`animate.rs:197`, `:209`,
  `:269`).
- **Dirty:** a slot that may be read first. It holds what an earlier frame left, or what
  Create New Object Instance wrote into the bottom frame's first local. It keeps its
  number.
- **Pinned:** a slot any other primitive names (Random, Set To Next, Relationship, ...).
  It keeps its number. Only `Expression` operands and Animate's event byte are rewritten.
- **No renaming at all** for a tree with any of these:
  - a `Dialog` or `ActionString`, which may format any local through `$Local:N` or
    `$TimeLocal:N`;
  - an unmodeled primitive, which may read the whole frame;
  - a call that takes inline arguments from the following nodes;
  - kind 4.

The renamed tree's frame uses and writes are re-derived from `primitive_frame_uses` and
`primitive_frame_writes`. If they differ from the original's renamed ones, the renaming
is dropped.

The behavioral relation observes no frame storage, and no read of a clean slot sees its
entry value. So the renamed tree reads and writes the same values in the same order.
Which permutation is tried (aligned to the other tree's first-touch order) affects only
whether a proof is found, never soundness.

## Decision 4: frame shape (V4)

The local count matters only to Create New Object Instance flag 0x02. It writes the new
object's bottom frame: the first argument, or the first local when there is no argument
(`create.rs:537`). So the check now compares local counts only for trees with no
argument:

- CharmMaker 4100 (1 argument, 2 → 14 locals) is accepted, as before, and soundly.
- Pool 4100 (no argument, 0 → 1 local) is still refused.

Accepting Pool 4100 would need a proof that the tree is never an object's bottom frame,
its Main entry. That proof would have to cover factory install, recovery, and saved
stacks. **Follow-up.**

## Decision 5: fault-time last result stays observed; opt-out is an assumption (V5)

The pilot's premise, "after an error exit the caller never reads the last result", does
not hold in sims-vm:

- A node sets `last_result` before branching (`interpreter.rs:1479`), and an error
  branch faults (`interpreter.rs:1646`).
- Play-mode recovery rebuilds an object's frame with the result its last primitive took
  (`visits.rs:280`).
- Recovery resets a person after copying that result into the person's last-action flags
  (`people.rs:1297`).

So by default it is still compared.

`verify-equiv --ignore-fault-last-result` (`LiveOptions::observe_fault_last_result`)
assumes nothing reads it after a fault. It prints an `ASSUME:` line saying what it
ignores.

## Decision 6: `--assume-equiv` and "EQUIVALENT modulo" (VR-3)

**`--assume-equiv <id[,id...]>`** assumes the listed private trees behave the same:

- Each applied tree's verdict becomes EQUIVALENT with "ASSUMED by --assume-equiv, not
  proven: …". This applies both as itself and as a tree another one may run.
- Each tree gets an `ASSUME:` line saying it is NOT PROVEN and that every dependent
  verdict holds only if it does.
- Only INCONCLUSIVE verdicts are replaced. A tree shown to differ, as itself or as a tree
  another may run, is never assumed. Neither is an ID that is not a private tree both
  files define. Each of these gets a NOT APPLIED line.

**Without the flag**, an INCONCLUSIVE run assumes every private tree that is not proven,
through the same code `--assume-equiv` uses. If every resource is then proven, the run
prints:

```
EQUIVALENT modulo unproven {#4112}: every other resource is proven if these trees behave the same (assume it with --assume-equiv 4112)
```

The set includes any tree the same run already assumed, so it is a complete
`--assume-equiv` list on its own. The exit code stays 2. A caller's INCONCLUSIVE reason
now lists every tree it waits on.

## Decision 7: unbounded loops (VR-2), not changed

Piano 4110/4118 loop on values no literal bounds. Widening those loops soundly needs
loop invariants over opaque reads, and this round does not attempt it.

Decision 1 already covers the case the pilot cared about: a wrong operand on the first
primitive behind such a loop is now reported.

## Consequences

- New synthetic tests are in `tests/equiv_pilot_upgrades.rs`. Adversarial negatives:
  - a pinned parameter slot is not DIFFERENT;
  - a stack object set to -1, or tested, before routing is not DIFFERENT;
  - a read that faults in every world (`global[-1]`) before routing is not DIFFERENT;
  - a tree ID with bit 15 set, or a kind-4 tree, is not DIFFERENT;
  - the modulo set is exactly what `--assume-equiv` proves, after a partial assumption
    too;
  - two global slots are not DIFFERENT;
  - a call before the primitive is not DIFFERENT;
  - an infeasible path is not DIFFERENT;
  - a read-before-write local is not renamed;
  - a `Dialog` blocks renaming;
  - a local Random names is pinned;
  - a renamed slot holding another value is not EQUIVALENT;
  - a fault-time last-result difference is not EQUIVALENT by default;
  - a different tree is never assumed equivalent.
- The local real-asset corpus keeps 0 counterexamples and 0 false equivalences.
