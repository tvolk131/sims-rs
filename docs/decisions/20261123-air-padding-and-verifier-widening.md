# `.air` padding clauses and a sound widening of the equivalence checkers

*Decided 2026-11-23.*

- **Scope:** `crates/sims-antics/src/{ast,parser,lower,structure_lift,verify}.rs`, `verify/{live,iff}.rs`
- **Builds on:** [`20261123-live-output-equivalence-verifier.md`](20261123-live-output-equivalence-verifier.md) and [`20261123-antics-instruction-layout-dedup.md`](20261123-antics-instruction-layout-dedup.md)

## Context

Strict lifting writes an instruction as raw `op 0xNNNN (b0, ..., b7)` whenever a
named `.air` form cannot rebuild its exact bytes. In the local corpus run, most of
these come from **padding**: bytes the VM never reads, holding values other than the
zeros the encoder writes. The five opcodes named in the task show it:

| Opcode | Primitive | Raw form left by strict lifting |
| --- | --- | --- |
| `0x0004` | Grab | all eight bytes `0xff` (operands ignored) |
| `0x000f` | Breakpoint | bytes 4..8 `0xff`; or every byte `0xff`, whose scope word is above 255 |
| `0x0019` | Alter Budget | selector 3 with an explicit scope byte; non-zero bytes 5 and 7 |
| `0x002c` | Animate | a local or event `param[0]` outside the tree's frame (not padding) |
| `0x0031` | Notify Idle | all eight bytes `0xff` (operands ignored) |
| `0x0002` | Expression | byte 4 (never read) not zero |

The verifier also rejected several pairs of trees that behave the same. The cases
asked about were:

- (a) calls whose operand bytes are never read;
- (b) unused reads of the stack object;
- (c) control-flow graphs that differ only by node numbering, local renaming, constant
  branches or unreachable nodes;
- (d) `--ignore-last-result` and `dependency_options`.

## Decision 1: one `pad` clause, checked against the VM's own reading

A primitive call may end in a `pad` clause:

```text
grab() pad 0xff
expr(temp[0] := 1) pad 0xff
alter_budget(local[0], category: 3, flags: 1) pad(0: 0x03, 7: 0x02)
```

- `pad 0xNN` sets every **free** byte to `NN`. A free byte is one where no value of
  it changes what the VM reads.
- `pad(i: 0xNN, ...)` sets the listed bytes. The list is in ascending order, each
  byte once, and each must differ from the plain encoding.

The compiler encodes the named form, applies the clause, and accepts the result only
if `canonicalize_primitive_operands` gives the same answer for it as for the plain
encoding (`verify::apply_operand_pad`). A clause that changes nothing is refused, as
is one that touches a byte the VM reads.

Strict lifting runs the same function in reverse (`verify::operand_pad_for`). When
the raw bytes do not decode, it decodes their canonical form and adds the clause that
rebuilds the raw bytes. It prefers `pad 0xNN`.

So padding has **no per-opcode table**. `canonicalize_primitive_operands` stays the
single statement of which bits the VM reads. It is the function the lockstep and
live checkers, and relaxed lifting, already use, and each of its arms cites its
`sims-vm` source line. The one change to it:

- **Expression byte 4** is now masked. `expression.rs` reads words 0 and 1 and bytes
  5 to 7, and nothing in `sims-vm` reads byte 4.

Because it is generic, the clause also spells padding on opcodes outside the five
above. The install round trip checks every such instruction byte for byte.

Raw `op 0xNNNN (...)` still compiles for every opcode.

**Breakpoint with a scope word above 255.** `debug.rs` converts the scope word to a
byte and fails with native error 9 before reading anything. The new form
`breakpoint(invalid_scope: 0xffff, index: -1)` spells it. The compiler accepts only
words above 255; smaller ones are ordinary operands.

**Expressions as conditions.** `expr(...)` is now accepted as a condition, so a
padded comparison can stand where `lhs == rhs` would.

**What stays raw, and why.** Only padding is spelled. An instruction whose read bytes
name something `.air` refuses stays `op`. Two examples:

- an Animate whose local or event `param[0]` lies outside the tree's frame;
- an Expression with an operator `.air` does not name.

Spelling either one would need the language to accept the frame-overrunning read
itself. That would weaken the frame checks every other tree relies on. The raw form
is exact and still verifies.

## Decision 2: argument-free calls compare without their operands (a)

Call setup in `sims-vm` (`interpreter.rs`, `Machine::call`) reads only two things
from a call's operand bytes:

1. the argument words, `argc` of them;
2. the entry word, which `entry_node` uses only for an indexed (kind 4) tree.

The all-`0xff` spelling reads the caller's temps instead. Temps always have eight
entries, so that read cannot fail either.

A callee with no arguments that is not kind 4 therefore reads none of its operand
bytes. `LiveContext::argless_callees` lists such trees. Both checkers compare a call
to one of them as `[0xff; 8]`, in events, in fault points, and in node-for-node
lockstep pairing.

The list is the intersection of the two sides' lists. For a whole file
(`check_iff_equivalent`), it holds:

- the file's own private trees that the VM keeps (the last copy wins);
- any library trees the caller names in `IffEquivOptions::extra_argless`
  (`library_argless_from_iff` builds that set from a library file).

Arguments that **spill** are unaffected. A call with more than four arguments reads
the instructions after it, and those raw bytes are still compared, even when the
instruction there is an argument-free call.

## Decision 3: reading the stack object is pure (b)

Scope 10, index 0 is a field of the running frame (`memory.rs`), so reading it never
fails and changes nothing. The live checker now gives it a symbol per *epoch*. An
epoch ends at any primitive or opaque write, so two reads agree only while nothing
in between could have moved the stack object. The read is no longer an event.
Index 1 and above still fault (native error 0xf) and stay events.

Frame residue is unchanged. A frame written to storage keeps its stack object, and
that comparison stays as it was.

## Decision 4: no node renumbering (c)

Lowering does not pair nodes up to renumbering. The frame header the VM writes on a
fault, a reset or a yield holds the node number (`interpreter.rs`,
`write_frame_to_storage`). The previous ADR already states that contract: primitive
events and fault points carry their node.

Three other things also make renumbering observable:

- any Expression node sets the scratch pointer;
- the last branch result can be read;
- the frame size, so a renamed local cannot be dropped.

What the live checker can show is kept: constant branches, unreachable nodes, and
dead pure nodes where nothing faults or yields. The new tests pin both sides:

- a renumbered sleep is not equivalent;
- a renumbered fault stays undecided;
- a folded constant branch is equivalent.

## Decision 5: the last result stays observed outside interaction tables (d)

Ignoring `last_result` for every tree not listed in a `TTAB` would be unsound.
`release_use` (`object_uses.rs`) reads the returning node's result when it pops the
frame of **any** tree entered through a counted object use, not only actions:

- Run Tree By Name in mode 2 (any named tree in any library's name table);
- functional trees;
- the portal, seat, route and feedback trees of go-to-slot;
- idle and found actions.

One file cannot rule any of these out.

`--ignore-last-result` and `--ignore-frame-residue` stay opt-in assumptions.
`dependency_options` keeps forcing both observations on every tree a primitive in
the checked reach can run. Relaxing that would let a padded or renumbered dependency
hide a difference that `release_use` makes visible.

Lowering `return false;` as a `0 != 0` test instead of the current node was tried
and reverted. A dummy self-assignment sets the last result to true, which the
existing `dummy_self_assignment_equals_return_false` policy pins, and the change
would rewrite every snapshot of `return false;` for no gain.

## Follow-up

`bin/sims-antics.rs` does not yet pass `library_argless_from_iff(Global.iff)` into
`IffEquivOptions::extra_argless`. The bin was outside this change's scope. Until it
does, CLI `verify-equiv` treats calls into global trees as reading their operands.
That is sound, just narrower.

## Reopen if

- `sims-vm` is found to read Expression byte 4, or any byte that
  `canonicalize_primitive_operands` masks. The `pad` clause and the checkers depend on
  that function together.
- The VM's call setup starts reading operand bytes for argument-free, non-indexed
  trees.
- Frame storage stops recording the node, which would make renumbering provable.
