# Live-output equivalence for `BHAV` trees, and whole-file `verify-equiv`

*Decided 2026-11-23.*

This is PR 3 of 3 of the Track 7 `.air` expansion. It follows
`20261122-air-expansion-wave1.md` and `20261123-air-expansion-wave2.md`.

## Context

The lockstep checker (`check_bhav_equivalent_with_arities`) is a node-for-node
product bisimulation. It treats every non-frame-private operand access as an
observable event, so it cannot prove that a tree equals a shorter one that
does the same thing. Maxis' editor left many such trees. "CT - Is Fridge"
(`Fridges.iff` tree 4143) is one node, `my.attr[0] > my.attr[0]`, whose two
branches both return true, so a high-level `return true;` should be provably
the same tree. Self assignments (`temp[0] := temp[0]`), temps that get
overwritten, and locals that nothing reads are similar.

One thing such a dummy tree does differ in is its own result. The VM keeps the
result of the last instruction a thread ran (`last_result`). When the frame of
the tree an action started pops, it records that result in the person's last
action (`sims-vm/src/object_uses.rs:246`). The Fridge tree's test is false, so
it leaves `false` where `return true;` leaves `true`. The checker reports that
by default (see Policies).

## Decision

The verdicts follow one rule: converge by narrowing the contract, not by
modeling more. Three review rounds of point fixes kept finding a new loader or
trap corner, so the fourth round made the contract structural, and the fifth
closed what it still let through (faults, callees, the replay, file bytes):
anything the checker does not model exactly is Inconclusive. The sixth round
replaced patching with two systematic passes over `sims-vm`: an inventory of
every error `run_tree` can reach ([Fault inventory](#fault-inventory)), and
callee resolution exactly as `program.rs` does it.

- **Equivalent** must be sound, under two stated model assumptions (below).
- **Different** (a counterexample) is reported only from a **concrete
  `sims-vm` replay**: both trees run from the same world and frame, and the
  runs observably differ. A symbolic difference alone is never enough.
- **Faults are never a difference.** Any node may fault: a modeled trap, a
  branch to the error exit or a missing node, and any primitive or call. A
  faulting path equals the other side's only when both fault at the same node,
  with the same instruction, from the same state before it; otherwise the
  pair is Inconclusive.
- **No error is assumed not to happen.** Every error `run_tree` can reach is
  either modeled exactly (with its whole error value), or a faulting path at
  its node, or makes the result Inconclusive. Anything a node may fail on gets
  a faulting path, over-approximating freely.
- **A tree is only as equivalent as the trees it may run.** Every tree in its
  reach must be the same bytes on both sides or itself proven equivalent.
- **Callees resolve as in the VM.** Only private trees (4096 to 8191) run from
  an object's file; 256 to 4095 come from the global library and 8192 and up
  from the semi-global one (`program.rs:1015`). A file's own copy of a library
  tree never supplies an argument count, a body or a replay, and `--tree`
  refuses one.
- **Whole files are compared only when they differ in BHAV bodies alone.**
  Every other byte must be identical: the file header, each chunk header
  (label field included), every non-BHAV payload, and the resource map apart
  from offsets that name the same chunks.
- Anything in between is **Inconclusive**. Fewer proofs is an accepted cost.

### Model assumptions

- **Instruction budget.** `Equivalent` means equivalent *when neither run
  exhausts the VM's per-tick instruction budget* (`interpreter.rs:494`). Dummy
  nodes change the instruction count by design, so the alternative, requiring
  equal instruction counts between observable points, would reject exactly the
  trees this checker exists to prove: every dummy node or self assignment it
  drops is one instruction fewer. The assumption is stated in this ADR, in `--help` for
  `equiv` and `verify-equiv`, and in every EQUIVALENT line they print.
- **World faults.** Faults the world raises on its own, independent of the
  tree's code, are outside the model, as they were for lockstep. The
  [Fault inventory](#fault-inventory) names them: the running object and its
  frame exist, its ID fits a script word (`memory.rs:365`), and the run does
  not start with an inline special panel waiting for an answer
  (`interpreter.rs:500`). Each would fail every operand or node alike, so a
  per-node faulting path would forbid every dummy node. Faults of the world
  that come before the first node (a full stack, no frame storage) are the
  same on both sides and need no assumption.

### 1. Live-output checker (`verify/live.rs`, `check_bhav_live_equivalent`)

Each tree runs on its own over symbolic inputs. Each path becomes a summary of
what the rest of the game can observe:

- **Exit:** true, false, error, a modeled VM trap with its native code, a
  jump to a missing node with the tree's length (both are in the VM's
  `InvalidBranch` error, `interpreter.rs:1484`), or an unmodeled fault of a
  primitive, call or operand. Branches to nodes 251 and 252, and kind-4 entries
  past node 250, trap with 0x44c whatever the tree's length
  (`interpreter.rs:766`, `interpreter.rs:1695`). A kind-4 entry runs at the
  entry clamped to the tree's last node (`interpreter.rs:1790`), so trees of
  different lengths are compared entry by entry, not rejected. The entry is
  also `param[0]` (`interpreter.rs:964`): the checker fixes it to the entry,
  or bounds it below for the last node, which every larger entry reaches.
- **Last branch result** at exit and before each primitive (a primitive may
  start a tree that reads it, `found_action.rs:102`, `idle.rs:187`). Before a
  built-in primitive other than Idle and Gosub Found Action, which read it
  first, a difference in it, the scratch pointer or the tracked cells is only
  possible: the primitive may overwrite them unread (`sound.rs:64`). The
  `>=`, `<=` and `!=` tests take their false branch when the canonical atom
  holds, so their own result is the atom's negation.
- **Event trace:** primitives and calls with canonical operands; reads of
  memory that may trap or has side effects; writes to memory the checker
  does not track. Each event carries a snapshot of what it may read:
  - the changed tracked cells,
  - the scratch-pointer state,
  - the frame slots the primitive reads. This uses the same per-opcode table
    as lockstep, now factored out as `primitive_frame_uses`;
  - the running frame's `(params, locals)`, on every primitive. Frame storage
    is one word stack, and any primitive may run a tree (a call, or a callback
    such as a routing door tree), whose frame starts after this one;
  - for calls, the callee's argument count in that side's file.
- **Reads of locals the tree has not written yet.** The VM can fail reading
  one, because it may hold half of a pointer an earlier frame left
  (`interpreter.rs:1277`). So such a read is an event, not a dead read.
- **Final values of the tracked cells.** These are `temp[0..8]` and
  `my.attr[0..N]`. Every object has at least 8 attributes
  (`sims-vm/src/factory.rs:314`), so N defaults to 8, and reads and writes of
  these cells never trap and have no side effects.
- **Scratch-pointer state at exit.** Every `Expression` sets it
  (`expression.rs:51`), and a caller's later read of Global 34 sees it.
- **Frame residue (on by default):** the frame's final params and locals,
  and which locals may be unwritten. A later frame at the same depth starts
  from these words and can read one before writing it
  (`interpreter.rs:1216`). A difference here is only ever possible, since it
  shows only if such a reader runs.

Values are named by what determines them. A name is one of:

- an initial input;
- a tracked cell at the start of an epoch, where an epoch is everything since
  the last event that may have written shared memory;
- an event's result, keyed by the whole event history before it, including
  store snapshots;
- a frame slot a primitive may have written, keyed by its old value;
- an operator applied to other values.

Equal names therefore mean equal values in both trees.

Two trees are equivalent when every pair of paths that can both run has the
same summary. A pair "can both run" when the conjunction of their path
conditions is not refuted by exact lookup or by interval reasoning on
term-versus-literal atoms.

A difference is a **candidate** only when all of these hold:

- the joint condition constrains independent inputs only by literal bounds,
  so concrete inputs that satisfy it can be picked. Inputs are only the
  tree's starting values: params, locals and tracked cells;
- no primitive runs before the paths diverge;
- the difference is in the exit, the last result at exit, the final tracked
  cells, or one trace having an event the other lacks;
- the differing values provably can differ.

A candidate becomes a **counterexample** only when a replay confirms it
(`verify/replay.rs`):

1. Up to two witnesses are picked from the joint condition (unbounded inputs
   zero, or distinct small values), each value the nearest one the condition
   allows. A kind-4 entry is `param[0]`.
2. Both trees run in `sims-vm` (`Interpreter::run_tree`) from the same
   concrete world: one object (the running object, a synthetic definition in
   library 1) with the witness temps and attributes, 64 globals, and frame
   storage holding the witness params and locals. The program is the one the
   VM builds for an object whose file holds the tree: the tree and the file's
   BCONs in the object's own library, an empty global library and no
   semi-global one. Only private trees have such a program; any other tree is
   not replayed, so its candidate stays Inconclusive.
3. Only trees whose every reachable node is an `Expression` are replayed. A
   primitive or call could reach state the replay world lacks.
4. The checker compares, field by field, only the plain words the replay
   defines and the VM reads back as stored: the return value, the running
   object's temps, attributes and data words, the globals, and (when
   observed) the object's last result. Words of different lengths are not
   compared. A difference anywhere else (frame storage, scratch pointers,
   records the world keeps in another representation) is not a
   counterexample. For example, writing a zero advertisement delta turns the
   world's "no advertisements yet" into sixteen zeroed ones, which the VM
   reads as the same zeros. The first difference is reported as
   "sims-vm runs both from params [..], ...: A returns true, B returns false".
   `run_tree` runs the tree as a synchronous call, after an eight-word call
   marker, so the witness params and locals are seeded from frame word 16
   (`interpreter.rs:1171`, `interpreter.rs:1217`). A unit test reads the
   witness frame back through a replay.
5. A run that faults confirms nothing. The checker stops replaying after 32
   candidates per tree pair.

A candidate the replay cannot run, or whose runs agree, is `Inconclusive`
("paths may differ, but no sims-vm replay confirmed it"). This removes the
whole false-counterexample class at once: opaque events, traps, raw kind
mismatches, and corners nobody has found yet.

One consequence: the replay world has no person and no action, so it cannot
observe the last result (`object_uses.rs:246` records it only for a person
with uses). A difference in the last result alone is therefore always
`Inconclusive`, not a counterexample.

Everything else is `Inconclusive` too. This includes:

- extra or different reads, which are unobservable unless they trap;
- differences between arithmetic terms;
- differences in what a primitive or called tree is handed (temps, scratch
  pointer, last result, frame operands, inline arguments), since it may not
  read them;
- frame residue;
- trees of different kinds (0 and 1 both enter at node 0,
  `interpreter.rs:1792`), and empty trees, which fault when entered.

**Faults.** A fault leaves the faulting frame on the stack at its node, with
whatever the node had done by then to the frame words, their unknown markers
and the last result (`interpreter.rs:809`). The checker does not model that
partial state. For example, `local := sqrt(-1)` clears the local's unknown
marker before it traps (`expression.rs:69`). Instead each faulting path
records a **fault point**: the tree's ID (a fault names its tree), the node,
its opcode and operands, and the whole
state before the node ran (the event history, tracked cells, scratch
pointer, last result, params, locals and which locals may be unwritten).
Everything the fault leaves is a function of these, so two faults are equal
only when their fault points are, whatever the observation options. Branch
targets are left out: a node that faults inside never reaches them, and one
that faults by branching shows its target in the exit.

**Primitives and calls may fault.** The checker does not model when (Random
with a zero range raises native error 4, `random.rs`; an operand may be bad,
an object or tree missing, a tree the primitive runs may trap). So every
primitive or call also has a faulting path. Its condition is a fresh atom of
the event, so the two trees agree on it only for the same event from the
same history, and the fault point compares the rest. A primitive event also
records its node, since a primitive that yields keeps the frame at that node
across ticks. The cost: a tree whose primitives move to other node numbers
(a dummy node before them) is no longer provable. That is the contract.

A kind-4 entry past node 250 is equal only to the same entry in a tree with
the same frame size.

What the checker models from `sims-vm`:

- **Expression:** its order of operations (right operand first, then the
  scratch pointer, then the left operand) and its traps:
  - 0x26 for division or remainder by zero;
  - 0x0d for a flag index out of range;
  - 0x13 for writing to a literal, checked before the zero check for `%=`;
  - out-of-range temp (2), param or local (8), and attribute (6);
  - scope 10 with a nonzero index (0x0f), and scopes 2, 5 and 39 to 255 (9);
  - square root of a negative number, which is reported as its own exit.
- **`++` and `--` compares** (ops 11 and 17).
- **Literal folding** and identities that hold for every 16-bit value. This
  lets literal-bounded loops unroll.
- **Owner BCON reads** (scope 26, tables 0..63, private trees) fold to
  literals when the file's constants are known.

- **Random** never takes its false branch: it succeeds or faults (`random.rs`).

Global 34 operands, and Random or GetDirectionTo with Global 34 or scope 22
aliasing, return `Inconclusive`. Kind-4 trees are checked from every entry
node.

### Fault inventory

Every error `Interpreter::run_tree` (`interpreter.rs:322`) can return for the
tree under check, and how the checker handles it. `run_tree` calls `run`
(`interpreter.rs:477`) with `most = usize::MAX`: it enters the tree, then
steps it until it returns, faults, yields or exhausts its budget. A
`Fault` is `{object, at: {tree, node}, trap}`, so each error names its tree
and node; every row below is compared with both.

Handling:

- **Exact:** the checker produces the VM's error, its fields included, at
  the same node, and compares it with the same rule as any fault. A native
  error's message is fixed by the site that raises it (0x26 says "division"
  or "remainder" by operator), and the fault point includes the instruction,
  so equal fault points carry equal messages.
- **Fault path:** the node gets a faulting path at that point, with the whole
  state (see `FaultPoint`). It equals the other side's only for the same
  node, instruction, events, store, scratch, last result and frame, so it
  never needs the error value.
- **Same on both sides:** the error depends only on inputs both runs share
  (the world, the caller's arguments, the tree's ID and header, all of which
  are checked equal first), and is raised before the tree's first node runs.
- **Inconclusive:** the checker gives up on the pair.
- **Assumption:** one of the two model assumptions above.
- **Cannot happen:** the VM's own invariants rule it out; the reason is given.

| Where (`sims-vm`) | Error | Handling |
|---|---|---|
| **Entering the tree** (`enter_as`, `interpreter.rs:931`; faults at node 0, before any node runs) | | |
| `objects.get(owner)` | native 0x15, missing object | Same on both sides |
| type table, type ID, `definition` | `Invalid` (GUID not in the type table, type ID too big, no definition) | Same on both sides |
| `behavior(owner, tree)` | `MissingTree`, `Invalid` for IDs 0 to 255 | Cannot happen: the tree under check is in the program. Only private trees are checked from a file, and the VM runs those from the object's own library (callee resolution below) |
| `dimensions` | `Invalid` "BHAV has no instructions" | Inconclusive (`check_bhav_live_equivalent`) |
| `dimensions` | `Unsupported` format outside 0x8000 to 0x8002 | Inconclusive (round 6) |
| `args.len() > 255` | `Invalid` | Same on both sides: different tree IDs are Inconclusive (round 6), so the fault is the same value |
| `argc > most` | `Unsupported` | Cannot happen: `run_tree` passes `most = usize::MAX` |
| `entry_node` | `Unsupported` negative kind-4 entry | Same on both sides (same ID and kind); a kind mismatch is Inconclusive |
| stack depth | `StackOverflow` | Same on both sides (the world's stack) |
| `push_frame` | `Invalid` "no frame storage", empty stack | Same on both sides |
| **Each node** (`step`, `interpreter.rs:1361`) | | |
| `current_cached` | native 0x44c for a node past 250 | Exact at entry: an entry past node 250 is equal only to the same entry of the same tree ID with the same frame, otherwise Inconclusive. Cannot happen after a branch: branches check their target first |
| `current_cached` | `Invalid` node outside the tree | Cannot happen: the entry is clamped to the last node and every branch checks its target |
| flagged opcode (`opcode & 0x8000`) | `Unsupported` | Fault path: any opcode other than 2 is a primitive or call to the checker |
| animator refresh, `refuse_unsettled_animator` | `Unsupported`, both-ways runs | Fault path for primitives. Cannot happen for Expression: `reads_no_animator(2)` (`placement.rs:239`) |
| `primitive_state != 0` | `Unsupported` resuming a primitive | Fault path for primitives. Cannot happen for Expression: a frame starts at 0 and every branch resets it (`interpreter.rs:1482`) |
| a primitive's error | any `Trap` | Fault path (every primitive, before and after its event) |
| a primitive's yield | native 0x44f, or `Ok(false)` with an unanswered inline special panel (`interpreter.rs:500`) | Fault path: the run ends at that node, from the same state on both sides or not at all |
| thread restarted by a primitive | `Unsupported` | Fault path |
| an inline special panel already unanswered when the run starts | every node, Expression included, ends the run with `Ok(false)` after it runs (`interpreter.rs:1442`) | Assumption (world faults) |
| branch to node ≤ 250 past the end | `InvalidBranch { target, count }`, after setting the last result | Exact: `Exit::BadNode(target, count)`, at the branching node with the last result (round 6 added `count`) |
| branch to 253 | `ErrorBranch` | Exact: `Exit::Error` at the branching node |
| branch to 251 or 252 | native 0x44c | Exact: `Exit::Trap(0x44c)` |
| return, then the caller's branch | `InvalidBranch` with the caller's `count`, `ErrorBranch`, 0x44c | Exact: the call node's own branch exits, as above |
| budget | `BudgetExhausted` | Assumption (instruction budget) |
| **Calls** (`call`, `interpreter.rs:1516`, opcode ≥ 256) | | |
| stack depth, `resolve_owner`, `behavior_from_owner` | `StackOverflow`, `Invalid`, `MissingTree` | Fault path at the call node |
| callee's `dimensions`, argument copy, `entry_node`, `push_frame` | `Invalid`, `Unsupported` (temps past 8, arguments read past the end of the BHAV), entry | Fault path at the call node. The argument bytes are compared too: by the callee's argument count when the VM's own library gives it, else the whole suffix of the tree |
| anything inside the callee | any | The callee is in the tree's reach: it must be the same bytes, or proven equivalent with its own faults compared by this table (cycles by induction). A callee from a library not supplied, or an undefined tree, puts every private tree in reach |
| **Operands** (`locate`, `memory.rs:364`, then `value` and `store`) | | |
| `frame()`, `me()`, the running object's ID fitting a word, `object(me)` | `Invalid`, native 10 | Assumption (world faults): they fail for every operand of every node alike, so a per-node fault path would forbid every dummy node |
| temp 0 to 7 (scope 8), `my` attribute 0 to 7 (scope 0) | none | Cannot happen: every object has 8 temps (`saved_objects.rs:384`) and at least 8 attributes (`factory.rs:314`, `objects.rs:530`). `--my-attributes N` states that the object has N |
| temp outside 0 to 7 | native 2 | Exact |
| `my` attribute index < 0 | native 6 | Exact |
| `my` attribute past the stated count | native 6 | Fault path (opaque read or write) |
| parameter or local out of range (scopes 9, 25, 22) | native 8 | Exact |
| scope 10 index ≠ 0 | native 0xf | Exact |
| scopes 2, 5, 39 to 255 | native 9 | Exact |
| a local the tree may not have written | `Unsupported` (`check_local`, `interpreter.rs:1277`) | Fault path at the read (round 6) |
| global 0 to 63 (scope 6) | native 0x3f out of range, `Invalid` on old saves with 32 globals | Fault path at the read or write (round 6) |
| global 34 | scratch-pointer aliasing, null read | Inconclusive |
| writing a global other than 34, 36, 3 (`memory.rs:684`) | native 0x13 | Fault path at the write (round 6) |
| every other scope (stack object, data, person, motive, slot, room, neighbor, definition, dynamic sprite, advertisement, ...) | native codes, `Invalid`, `Unsupported` (motive conversions, unknown person words, data words 2, 3, 11) | Fault path at the read and at the write (round 6). The event records the whole store, so a temp the VM indexes by (scopes 11, 17, 23, 27) is compared |
| BCON (scope 26), the file's own table | `Invalid` "no tuning constant" | Exact when the file's constants are known: a constant reads as its value, a missing one is an opaque read with a fault path. The files' tables must be equal when either tree may read one |
| BCON in the semi-global or global tables (64 to 191) | `Invalid` | Fault path (opaque read) |
| **Expression** (`expression.rs:25`), after its operands | | |
| right operand Global 34 | `Unsupported` | Inconclusive |
| op 7 or 8 (divide, remainder) by zero | native 0x26 | Exact |
| op 9 or 10 flag index outside 0 to 15 (0 to 16 on motives) | native 0x0d | Exact for integer destinations; fault path (opaque write) for motives |
| op 12 on a literal | native 0x13, before the 0x26 check | Exact |
| ops 11 and 17 on a literal | native 0x13 | Exact |
| op 20 of a negative number | `Unsupported` | Exact (`Exit::Trap(NEGATIVE_SQRT)`, its own exit) |
| an operator above 20 | native 0x01 | Inconclusive |
| any write to a literal | native 0x13 | Exact |
| motive destinations | float arithmetic, clamps, 0x13 for integer-only operators, `Unsupported` for ++ and -- | Fault path at the read and the write; the value written is an event, so both sides must write the same thing |

### 2. Combined checker (`check_bhav_equivalent_combined`)

The combined checker runs lockstep first and falls back to live-output. It
reports which method decided (`EquivMethod`).

- If the two files' BCON tables differ and either tree may read them (any
  primitive, or a scope-26 operand), the result is `Inconclusive`: neither
  checker sees constants a primitive or an unfolded operand reads
  (`context_mismatch`). The live checker applies the same rule when called
  on its own.
- Lockstep runs with silent steps off (`max_silent_steps: 0`), so every
  `Expression` is matched. That keeps the scratch pointer and unwritten-local
  reads in step. With `observe_last_result`, it also tracks whether the two
  sides' own results may differ (`EquivOptions::observe_last_result`).
- Its proof is taken only when it covers everything live-output observes
  (`lockstep_suffices`):
  - lockstep paired every node with the node of the same number
    (`LockstepTrace`), and every node it ran is the same bytes on both sides,
    a call with the same inline arguments after it too. Lockstep matches what
    nodes do, not how; two nodes that compute the same thing differently can
    fault differently. With the same bytes at the same numbers, the two runs
    are the same run;
  - the trees have the same ID, format, kind and length (round 6). A fault
    names its tree, and a branch past the end reports the tree's length;
  - the frames have the same size. With silent steps off, lockstep matches
    every frame write, so equal frames end with equal residue;
  - when the last result is observed, it never differed between the sides
    mid-run, even where lockstep's own check would let it;
  - no tree has more than 251 nodes or branches to 251 or 252;
  - no called tree's argument count differs between the files;
  - no path may read a local before writing it. Lockstep folds
    `local[0] == local[0]` to `true`, but the read can fail
    (`interpreter.rs:1277`). A must-be-written analysis decides that,
    counting only `:=` and op 20 as writes.

A lockstep counterexample is a divergence in the stricter
all-accesses-observable trace and is never reported. Only a live-output
counterexample, which a replay confirmed, is. Otherwise the result is
`Inconclusive`, with the lockstep note attached.

`sims-antics equiv` now uses this checker, through `check_tree_with_callees`.
Before it extracts the target tree, it applies the single-file precondition
from section 3 to the IFF: no two chunks of the same type and ID, a resource
map that selects every chunk the loader keeps, or no map. A file that fails it
is INCONCLUSIVE. The checker runs with each side's own callee argument counts:
the compiled source's trees for A and the IFF's for B. A callee only one side
defines has an unknown count on the other. So a mutated script whose tree
live-output cannot bound comes out INCONCLUSIVE (still a non-zero exit), with
the lockstep divergence named. One example is the open-pack toilet's motive
loop. `tests/open_pack_household_sim.rs` now accepts that.

Both checkers treat a call as an event: equal calls from equal states do the
same thing only if the trees they run do. So after the target tree is proven,
`equiv` also checks its **reach** (`tree_reach`): every tree it may run. A
call enters its callee. A primitive, or a call to a tree neither side defines,
may run any tree of the object (Run Tree By Name, functional trees, found
actions, Idle For Input, routing callbacks, Create, ...), so then every tree
of both sides is in reach. Each tree in reach must be defined on both sides
and be the same bytes (header words and instructions) or be proven equivalent
by the combined checker. Otherwise the result is INCONCLUSIVE.

The review asked for "proven, with no cycles". This deliberately allows cycles,
because any primitive may run any tree: a tree with a primitive has itself in
reach, so "no cycles" would leave every tree with a primitive Inconclusive.
Cycles are sound here by induction on the length of a run within the
instruction budget. Assume every tree in a closed reach was proven
equivalent, assuming the trees it runs behave the same. A call inside a run
of length n is a shorter run, so it behaves the same by induction, and so
does the run.

When a run may read other resources of its file (a primitive, a call into the
game, or a tuning constant), both sides must be IFFs that differ in BHAV
bodies alone (section 3). A v2 source has its compiled IFF; an `.air` source
has none, so such a tree is INCONCLUSIVE from `.air`. Each side's BCONs are
used when both sides have a file.

`lift --verify-equiv` still uses lockstep alone, with its defaults (silent
steps on, last result not compared), because a structured lift should match
node for node.

### 3. Whole-file checker (`check_iff_equivalent`, `sims-antics verify-equiv`)

Whole-file mode is strict about structure. The verdict is `Inconclusive`
unless all of the following hold:

- **Each file on its own** (`single_file_problem`, also used by `equiv`):
  - its resource map, if any, parses;
  - no two physical chunks share a type and ID;
  - every chunk other than `rsmp` is the one `Iff::get` selects for its type
    and ID.
- **The two files together** (`structure_problem`):
  - they have the same number of chunks, in the same physical order, each
    with the same type and ID. An IFF is its 64-byte header and then its
    chunks back to back (`Iff::parse` rejects anything else), so the rules
    below cover every byte;
  - the file header is the same bytes, except that its resource-map offset
    (bytes 60..64) may differ when both name the same chunk. A map offset of
    zero turns the map off, so a map one file points at and the other does
    not is a difference;
  - each chunk's 76-byte header is the same bytes: tag, ID, flags and the
    whole 64-byte label field, bytes after the terminator included. Only a
    `BHAV`'s size may differ. The effective resource-map flags must match
    too;
  - every non-`BHAV` payload is byte-identical;
  - the resource maps are the same bytes, except offsets that name the same
    chunk. A walk that follows the `sims-formats` map parser field for field
    replaces each offset by the physical index of the chunk it names and
    copies every other byte (the unused word, labels, pad bytes) as it is.
    A map it cannot walk compares by its raw bytes.

Then only `BHAV` bodies can differ. Each private tree (4096 to 8191) goes
through the combined checker, with its file's BCONs (identical in both files
by the rule above) and the argument counts of the trees its calls run:

- **Callees resolve as `program.rs:1015` does.** A call to 4096 to 8191 runs
  the object's own tree, so the file supplies it. A call to 256 to 4095 runs
  the global library's tree and one to 8192 and up the semi-global one's,
  whatever the file holds under that ID; their argument counts come only
  from `--global <Global.iff>` and `--semi-global <file.iff>` (the old
  `--with` took any file's trees for any ID). A file's own chunk in those
  ranges never supplies an argument count, a body or a replay.
- **A callee whose argument count is unknown** (its library not supplied, or
  an undefined tree) is compared conservatively: the call node and every node
  after it must be the same bytes on both sides, which covers every argument
  count the VM could read. Its body is outside both files, but both runs use
  the same game's library, so it may run anything: every private tree of the
  file is then in the tree's reach. This is a judgement: making every such
  call Inconclusive would lose every tree that calls the game library, and
  the suffix rule plus the reach rule are sound for any callee.
- **Trees in the library ranges** in an object's file are never run by the
  VM from that file, so they compare as bytes only: the same bytes are
  Identical, anything else is Inconclusive with the reason. `--tree` refuses
  them (exit 2).

A tree whose argument count differs is `Inconclusive`. A tree that is the same bytes, or proven
equivalent, is reported so only when every tree in its reach is too (see
section 2). Otherwise it is `Inconclusive` and names the tree it may run.

This one rule replaced the per-type comparisons of earlier rounds (string
tables as the loader reads them, BCON values, editor metadata as notes,
duplicate-copy matching, resource-map selection). Each of those kept finding
a new loader corner; byte identity has none. A file pair that differs in
anything but trees is not what this tool is for.

The verdict is Different if any tree is (replay-confirmed), otherwise
Inconclusive if anything is undecided or a precondition fails, otherwise
Equivalent. Reports name resources and positions only, never contents.

With `--tree`, only that tree is reported, but the structure rule still
applies, since the tree may read constants, strings and tables. The trees in
its reach are checked too, and it is `Inconclusive` unless they all behave
the same. The CLI exits 0, 1 or 2 for
these, and with 2 on usage or I/O errors. The CLI flags are:

- `--tree <id>` (a private tree, 4096 to 8191)
- `--global <iff>` and `--semi-global <iff>` (repeatable)
- `--my-attributes <n>` (at least 8)
- `--ignore-scratch-pointer`
- `--ignore-last-result` (also on `equiv`)
- `--ignore-frame-residue` (also on `equiv`)
- `--all`

## Policies and why

- **Frame residue is observed by default.** A frame's final locals become
  the next same-depth frame's initial locals (`interpreter.rs:1216`), and an
  identical later reader can see what differs. A difference in it is only
  ever `Inconclusive`. `--ignore-frame-residue`
  (`LiveOptions::observe_frame_residue = false`) relaxes it for callers that
  know no later frame reads a local first. Dead local writes are therefore
  undecided by default.
- **The scratch pointer is observed by default.** A dummy `Expression`
  placed before a primitive changes what a later Global 34 read sees. The
  Fridge tree and `return true;` both end in one `Expression`, so they agree.
  `--ignore-scratch-pointer` relaxes this.
- **The last branch result is observed by default**, because the VM records
  it (`object_uses.rs:246`). This keeps the Fridge tree from being proven
  equal to `return true;`, and a loop that returns from its failed exit test
  from being proven equal to straight-line code. Such a difference is
  `Inconclusive`, never a counterexample, since the replay world cannot see
  it. `--ignore-last-result` (`LiveOptions::observe_last_result = false`)
  proves such trees equivalent. Use it when the tree is not the root of an
  action, as a check tree is.
- **Attributes 8 and up are opaque** unless `--my-attributes` says the
  running object has them. Reading them can trap (native 6).

## Dependencies and tests

`sims-antics` now depends on `sims-vm`, `sims-sim` and `sims-content` (for the
replay), not only as dev-dependencies. `sims-vm` stays a read-only reference:
the replay uses its public API only.

`tests/live_equiv.rs` pins the contract:

- the tests that already passed on the parent stay as baseline coverage;
- `call_arity_is_checked_before_operands` pins the precedence (differing
  operands to a call whose arity differs);
- `counterexamples_come_from_a_vm_replay`, `faults_keep_their_state`,
  `renumbered_faulting_nodes_are_undecided`,
  `instruction_budget_is_a_model_assumption`,
  `non_bhav_chunks_must_be_identical`,
  `resource_maps_compare_by_the_chunks_they_select` and
  `equiv_requires_a_single_copy` pin the round-4 rules;
- round 5 adds each of Sol's repros as a test:
  - `primitives_may_fault_at_their_node`: Random with a zero range at node 1
    vs node 2, and after different last results;
  - `faults_keep_their_state`: different last results before the same
    division with `--ignore-last-result`, and `sqrt(-1)` into a local vs a
    temp;
  - `equiv_checks_the_trees_a_tree_calls` and
    `whole_file_trees_depend_on_their_callees`;
  - `replay_ignores_representation_changes` (a zero advertisement delta);
  - `replays_seed_locals_and_temps`, plus the `replay.rs` unit test
    `replay_frame_reads_back_the_witness`;
  - `file_and_chunk_headers_compare_byte_for_byte`: a map one header points
    at, and a label byte after the NUL;
- round 6 adds each of Sol's repros, plus the inventory's new rules:
  - `failing_operands_fault_at_their_node`: `temp[0] = global[64]` after
    one node vs two, an unwritten local read and a `global[5]` write at
    different nodes, and the same trap in trees 4096 and 4097;
  - `missing_node_faults_carry_the_tree_length`: a jump to node 2 of a
    one-node vs a two-node tree, live and combined, and format 0x8003;
  - `library_trees_in_an_object_file_supply_nothing`: an embedded #256 does
    not supply the callee's argument count, `--global`'s does, and a private
    callee that differs is named;
  - `library_range_trees_are_never_replayed`: #256 differing is
    Inconclusive, `--tree 256` is refused (also on the CLI, exit 2), and only
    a private tree's difference is replayed into a counterexample;
  - `recursive_call_graphs`: 4096 and 4097 calling each other, proven by
    induction, and both undecided once one of them differs;
  - `different_tree_ids_never_fault_alike`: the same bytes under 4096 and
    4097, live and combined (lockstep pairs every node).
  Each round-6 guard was removed in turn (14 mutations: the opaque-read,
  unwritten-local and opaque-write fault paths, the bad branch's count, the
  lockstep header rule, the format check, the fault's tree, the different-ID
  rule, private-only arities and reach, the replay library, `--tree`'s
  refusal, byte-only library trees, and the callee rule on cycles), and each
  made at least one of these tests fail;
- the round-5 P2 fixtures fail when their guard is removed. This was checked
  by removing each guard in turn: the fault-point comparison, the primitive
  fault path, the fault point's last result and instruction, the replay's
  frame offset and field comparison, the label, header and map comparisons,
  and both callee rules:
  - `renumbered_faulting_nodes_are_undecided` uses the same node-0 test on
    both sides, whose targets lead to their own divisions, numbered the
    other way round in B;
  - `resource_maps_compare_by_the_chunks_they_select` keeps the map and
    changes only its unused word, then an entry's pad byte;
- tests that expected a counterexample from a last result, a trap, a raw
  kind or a non-`BHAV` chunk now expect `Inconclusive`.

## Results (Complete Collection, `SIMS_INSTALL`, in memory, counts only)

`tests/live_equiv_corpus.rs`, default options (last result observed), after
the round-6 fixes (round-5 numbers in brackets):

- **Self-check:** 28,690 trees, each checked against itself.
  - **16,650 Equivalent** [16,685]. The 35 lost exceed a budget: every
    untracked operand now adds a faulting path too.
  - 12,040 Inconclusive. Of these, 9,478 have a loop whose exit depends on an
    event or input, 2,289 exceed 4,096 paths, 140 exceed the pair budget, 124
    are empty and 9 use Global 34.
  - **0 counterexamples.**
- **Structured lift, recompiled:** of the 27,781 trees lockstep proves
  equivalent, live-output agrees on **7,784** [9,636], is Inconclusive on
  19,997, and **contradicts it on 0**. The lost trees are lifts that move a
  fallible node to another node number. Any primitive, call or untracked
  operand may fault, and a fault leaves its node on the stack, so the node is
  observable: 6,944 Inconclusive results name a primitive or call at
  different nodes, 1,851 an operand (new in round 6), 12 a trap and 6 an
  error exit. This is the intended effect of the fault rule. Lockstep still
  proves these trees.
- **One-node dummy compares** whose branches agree: 309 trees.
  - Last result observed: 46 proven, 172 undecided on the last result alone,
    0 counterexamples.
  - Last result ignored: 218 proven equal to `return true;` or
    `return false;`, 0 counterexamples, and the rest read memory that can
    trap. **`Fridges.iff` tree 4143 is proven equal to `return true;`.**

## Limits, and what would reopen this

Path enumeration cannot summarize a loop whose exit depends on an event (such
as `while` around a call, or idle and retry), so these trees are
`Inconclusive` in live-output. The combined checker still proves them
whenever lockstep does. Closing them needs an event-aligned product
bisimulation with symbol renaming at loop heads, which would memoize
(pcA, pcB, canonical frame) after each pair of matched events. That is the
natural follow-up if a migration needs dummy-node elimination inside such
loops.
