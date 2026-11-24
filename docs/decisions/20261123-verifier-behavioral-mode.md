# Opt-in behavioral equivalence, and cut-point bisimulation for loops

*Decided 2026-11-23.*

- **Scope:** `crates/sims-antics/src/verify/live.rs`, tree reach in `verify/iff.rs`, and the `verify-equiv` flag in `bin/sims-antics.rs`
- **Builds on:** [`20261123-live-output-equivalence-verifier.md`](20261123-live-output-equivalence-verifier.md) and [`20261123-air-padding-and-verifier-widening.md`](20261123-air-padding-and-verifier-widening.md) (Decision 4 there: node numbers are observable)

## Context

`verify-equiv` proves that two trees cannot be told apart by anything the VM keeps.
That includes **frame storage**:

- the node number a fault, a reset or a yield records;
- the locals a tree leaves behind for a later frame;
- the offset at which a callee's frame starts.

Two compilers that lay out the same program differently therefore never prove
equivalent, even when every effect on the world is the same. A synthetic example: a
tree lifted to `.air` and compiled straight back gets new node numbers.

The live checker also enumerates whole paths. It gives up in two cases:

- a loop whose exit does not depend on literals (wait for an animation event, scan
  objects until none is left, idle until interrupted);
- a chain of choices whose paths multiply past 4096, such as a retry loop with several
  primitives per try. Each primitive has three outcomes: true, false and fault.

## Decision 1: the behavioral relation, opt-in

`LiveOptions::behavioral` (`verify-equiv --behavioral`) compares trees under a weaker,
explicitly stated relation. The default stays strict and is unchanged.

**Still observed under `--behavioral`, exactly as in strict mode:**

- **Primitives and calls:** the ordered trace of every primitive and call. Each entry
  includes:
  - the opcode and the canonical operands;
  - the inline arguments a call reads;
  - the frame values the primitive reads, and for a frame operand outside the frame,
    how far past its end it falls. Primitives check that bound themselves and fail
    differently one slot past the end than further out (Push Interaction, `push.rs`);
  - a snapshot taken before it runs: the tracked cells, the scratch pointer and the last
    branch result.
- **Expressions:** reads and writes of memory the checker does not track. That covers
  object, person and world data, motives, globals and the stack object's fields.
- **Exit:** how the tree exits (`true`, `false`, `error`), the final temps and
  attributes, the scratch pointer and the last branch result.
- **Faults:** whether a path faults at all, and what kind of fault it is:
  - the trap code;
  - a failing primitive, together with its event;
  - a failing operand read;
  - a branch to a missing node.

  What the fault leaves behind is compared too: the event history up to the failing
  event, the tracked cells, the scratch pointer and the last result, all at the moment
  of the fault. These are compared even under `--ignore-scratch-pointer` and
  `--ignore-last-result`, because a fault leaves them in the object
  (`interpreter.rs:809`).

  Strict mode does not trust the checker's model of a node's partial effects, so it
  compares the node's bytes and the whole state before the node. The behavioral
  relation does rely on that model, which applies them in the VM's order:
  - an `Expression` reads its right operand, then sets the scratch pointer, then
    resolves and reads its left one (`expression.rs:43`, `expression.rs:51`);
  - it writes its destination last;
  - a node sets the last result only when it succeeds (`interpreter.rs:1479`).
- **Frame shape:** the argument count, and whether the frame has any local. This holds in
  both modes, and is checked before any path is explored. The counts compared are the
  ones the VM uses: the format caps them and the local count is a byte. Two reasons:
  - a caller supplies only so many arguments;
  - Create New Object Instance (flag 0x02) writes the first word of the new object's
    bottom frame, its first argument or else its first local, and fails when there is
    neither (`create.rs:524`).
- **The scratch pointer after a write to untracked memory:** it is whatever the write
  left, not the state the `Expression` set before storing. A store can run code that
  leaves the scratch pointer in another state: selecting a Sim runs its placement
  callback (`memory.rs:846`). This holds in both modes. Strict mode assumed the
  `Expression` state before this change.
- **Flags:** `--ignore-last-result` and `--ignore-scratch-pointer` keep their meaning and
  can be combined with `--behavioral`.
- **Whole-file runs:** trees in another tree's reach still always observe the last result
  (`dependency_options`), because `release_use` reads it.

**Abstracted under `--behavioral`:**

1. **Node numbers.** The checker drops the node a primitive runs at, the node a fault
   happens at, and the target and tree length in an invalid-branch error.
2. **Frame storage.** The checker drops:
   - the final parameters and locals (frame residue; `--behavioral` implies
     `--ignore-frame-residue`);
   - the frame contents when any node faults;
   - the caller's frame size at a call, which fixes the callee's frame offset;
   - the number of locals beyond the first.

**What a behavioral proof does not cover:**

- **Saved frames across builds.** A frame saved by one build and resumed by the other
  resumes at a node number that means something else there. Behavioral equivalence is a
  claim about runs that start and end under the same build.
- **Stale frame words.** A tree that reads a local it has not written reads a word an
  earlier frame left behind. That includes a callee whose frame starts where its
  caller's frame ends. Under `--behavioral` that word is assumed to be the same unknown
  on both sides at the same position in the tree's own frame. This holds when no tree
  reads a local before writing it.

  The checker still marks such a read as an event that may fail. It does not prove that
  the two builds leave the same word there. Strict mode observes the frame layout and
  needs no such assumption.

Because a local's number never reaches the world, local renumbering drops out with
frame storage. Dead locals and different local counts drop out the same way. The
exception is a primitive operand that names a local: its bytes are still compared.

The lockstep checker is unchanged. It still proves only trees whose nodes pair up at the
same numbers. Strict equivalence implies behavioral equivalence, so its proofs stand in
both modes. The live checker proves renumbered trees.

## Decision 2: Animate's event local is a dead byte in live events without flag 0x20

Animate reads operand byte 2 only when flag 0x20 sends events to a local
(`animate.rs:271`). Otherwise the event goes to `param[0]`.

- **Live checker:** the primitive events it compares zero the byte when the flag is
  clear, and it writes only the slot the flag selects. This holds in both modes.
- **Strict fault snapshots:** these still compare the node's raw bytes.
- **Canonical operands are unchanged:** `canonicalize_primitive_operands` is what `pad`
  clauses and lifting use, so `animate(...) pad 0xff` still fills only bytes 3 and 7.
  - Changing it would change the bytes that existing `.air` compiles to.
  - A call before the node may read byte 2 as an inline argument. The call's event
    records that read.

## Decision 3: cut-point bisimulation when path enumeration gives up

When either tree exceeds the path or step bound, the live checker now tries a
bisimulation that cuts both runs at primitive events. This applies in both modes.

1. **Segments.** From a product state, each side runs symbolically until it has run one
   primitive or call, or has exited or faulted. A branch from that event to an exit or
   a missing node finishes inside the segment, so a strict fault there still records
   its node. Expression reads and writes of untracked
   memory stay inside the segment and are part of its trace.

   If that proof fails, the checker tries again and cuts after every node that recorded
   an event, including untracked reads and writes. That merges chains of diamonds that
   branch on untracked memory and run no primitive. It is a fallback because it needs
   both trees to split their events over nodes the same way.
2. **Pairing.** Take every pair of segment ends (one from A, one from B) whose path
   conditions can hold together. One of two things must be true:
   - both end in the same way, with identical outputs (exit, fault, trace, cells,
     frames);
   - both ran the same event and took the same outcome.

   In the second case, the two states after the event become a new product state.
   Anything else is `Inconclusive`. This mode never reports a counterexample.
3. **Abstraction.** A product state keeps only the following:
   - both node numbers;
   - both last results and both scratch-pointer states;
   - which locals may still be unwritten;
   - for every frame slot on both sides, and every tracked cell either side has changed
     in the current epoch, either its literal value or an equivalence class.

   Two slots share a class only when they hold the same symbolic value. Class symbols are
   fresh unknowns, so the abstract state stands for every concrete pair of states with
   those equalities and constants. That makes it an over-approximation. The path
   condition is dropped, which loses precision but never soundness.

   Everything else in shared memory is the same on both sides, because both runs have
   the same history. Each product state starts a fresh root, so the next segment names
   unchanged cells, the stack object and untracked memory by that root on both sides
   alike. After a primitive the tracked cells are already fresh, so the cell list is
   empty.
4. **Merging.** Product states with the same abstract state are explored once. This
   both closes loops and merges the diamonds that primitives create.

   Exploration has to end. After four visits to a pair of nodes, every slot whose literal
   has not stayed the same on each visit is widened into a class, with equal literals
   sharing a class. A counter that no literal bounds then stops unrolling. There is also
   a cap of 4096 product states, after which the checker gives up.

**Why it is sound.** The argument is induction on the number of events. Both runs start
from the same inputs. Assume two concrete runs reach a product state's nodes with:

- equal histories;
- frames that match its equalities and literals.

Every pair of their next segments that can happen together is then checked to:

- end equally;
- or run the same event to the same outcome and land in a product state that covers
  them.

So every run's trace and outputs equal the other's, up to the event count. The
instruction-budget assumption is the same as in the live checker.

Two ingredients make this hold:
- Symbolic execution of a segment covers every concrete run from the product state,
  just as it does from the entry.
- Pairs are dropped only when their path conditions contradict each other. Those
  conditions are stated over values both sides name alike: classes, the root's cells,
  and values the segment's events return.

In strict mode, events, faults and outputs still carry node numbers and frame residue,
so the bisimulation proves only what strict mode can observe.

**What it still cannot do:**

- A loop with no primitive in its body is still unrolled within a segment.
- A chain of pure `Expression` diamonds between two events still multiplies paths within
  that segment.

Both still end in `Inconclusive` past the bounds.

## Decision 4: whole files

`check_iff_equivalent` already reports a file equivalent when every tree in every reach
is the same bytes or proven equivalent. `IffEquivOptions::live.behavioral` flows into
every comparison it makes, including the stricter dependency comparisons. So a file
whose trees all prove behaviorally equivalent reports `EQUIVALENT` under `--behavioral`.

A tree's reach now also counts a store to global 3, the selected Sim, as able to run any
tree of the file, as a primitive is. In co-op that store selects the Sim, and selecting
can run object callbacks (`memory.rs:846`). Reads and comparisons of global 3 do not
count. This holds in both modes and was missing before this change.

## Reopen if

- Saves need to load across builds. In that case node numbers and frame layout become
  part of the contract again, and `--behavioral` must not be used to accept a build.
- `sims-vm` is found to expose node numbers or frame storage to the world in a way other
  than a saved, faulted or resumed frame.
