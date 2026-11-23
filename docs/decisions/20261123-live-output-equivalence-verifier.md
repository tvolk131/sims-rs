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

The verdicts follow one rule, adopted after three review rounds kept finding
new holes: converge by being conservative, not by modeling more.

- **Equivalent** must be sound.
- **Different** (a counterexample) must be backed by a witness the VM can
  reach.
- Anything in between is **Inconclusive**. Fewer proofs is an accepted cost.

### 1. Live-output checker (`verify/live.rs`, `check_bhav_live_equivalent`)

Each tree runs on its own over symbolic inputs. Each path becomes a summary of
what the rest of the game can observe:

- **Exit:** true, false, error, a modeled VM trap with its native code, or a
  jump to a missing node. Branches to nodes 251 and 252, and kind-4 entries
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

A difference becomes a **counterexample** only when both of these hold:

- the joint condition constrains independent inputs only by literal bounds.
  Inputs are only the tree's starting values: params, locals and tracked
  cells. A value a primitive produced can be constrained by what it does (a
  `Random` result is below its bound), and a read of unmodeled memory may
  only ever return some values (`memory.rs:542`), so neither is free;
- no primitive runs before the paths diverge. A primitive both paths ran may
  never succeed, or never take the branch they took after it (a `Random`
  with a zero bound always fails);
- the difference is in the exit, the last result at exit, the final tracked
  cells, or one trace having an event the other lacks. Two different events
  at the same point are only possibly different: both may fail before doing
  anything, as two `Random`s with a zero bound do. So are differences in what
  a primitive or a called tree is handed (temps, scratch pointer, last
  result, frame operands, inline arguments), since it may not read them, and
  frame residue;
- the differing values provably can differ.

Anything else is `Inconclusive`. This includes extra or different reads,
which are unobservable unless they trap, and differences between arithmetic
terms.

What the checker models from `sims-vm`:

- **Expression:** its order of operations (right operand first, then the
  scratch pointer, then the left operand) and its traps:
  - 0x26 for division or remainder by zero;
  - 0x0d for a flag index out of range;
  - 0x13 for writing to a literal, checked before the zero check for `%=`;
  - out-of-range temp (2), param or local (8), and attribute (6);
  - square root of a negative number, which is reported as its own exit.
- **`++` and `--` compares** (ops 11 and 17).
- **Literal folding** and identities that hold for every 16-bit value. This
  lets literal-bounded loops unroll.
- **Owner BCON reads** (scope 26, tables 0..63, private trees) fold to
  literals when the file's constants are known.

- **Random** never takes its false branch: it succeeds or traps (`random.rs`).

Global 34 operands, and Random or GetDirectionTo with Global 34 or scope 22
aliasing, return `Inconclusive`. Kind-4 trees are checked from every entry
node.

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
  (`lockstep_suffices`). With silent steps off, lockstep matches every frame
  write, so equal frames end with equal residue. It is not taken when the
  frames differ in size and residue is observed or a tree has a primitive, when a tree has more than 251
  nodes or branches to 251 or 252, or when a called tree's argument count
  differs between the files. Nor is it taken when a path may read a local
  before writing it: lockstep folds `local[0] == local[0]` to `true`, but the
  read can fail (`interpreter.rs:1277`). A must-be-written analysis decides
  that, counting only `:=` and op 20 as writes.

A lockstep counterexample is a divergence in the stricter
all-accesses-observable trace. When live-output cannot decide, the result is
`Inconclusive`, with the lockstep note attached, rather than "different".

`sims-antics equiv` now uses this checker, with each side's own callee
argument counts: the compiled source's trees for A and the IFF's for B. A
callee only one side defines has an unknown count on the other. So a mutated script whose tree
live-output cannot bound comes out INCONCLUSIVE (still a non-zero exit), with
the lockstep divergence named. One example is the open-pack toilet's motive
loop. `tests/open_pack_household_sim.rs` now accepts that.

`lift --verify-equiv` still uses lockstep alone, with its defaults (silent
steps on, last result not compared), because a structured lift should match
node for node.

### 3. Whole-file checker (`check_iff_equivalent`, `sims-antics verify-equiv`)

Resources are matched by `(type, id)` the way the original looks them up
(`Iff::get`). Deleted chunks and `rsmp` are skipped. Each side's callee
arities and BCON tables come from the chunks it resolves.

| Resource | How it is compared |
|---|---|
| `BHAV` | Combined checker. Each side uses its own file's arities and BCONs, plus any `--with <file.iff>` arities. Identical bytes are `Identical` only when the tree's context matches too (BCONs it may read, callees' arities). A label difference is a difference: Run Tree By Name looks trees up by label (`sims-vm/src/program.rs:607`). An argument-count mismatch is a difference. Locals, format, revision and trailing bytes are notes. |
| `STR#`, `CTSS`, `TTAs` | Parsed with the chunk's byte-order flag, then compared as the original's loader reads them. Version-0 tables load empty (0x4700d7). Length-prefixed strings get 0xa1 → `o` and 0xd5 → `'` (0x4700f5). Language-tagged tables must match language and text in order, and a change to or from that layout is `Inconclusive`. Layout version and comments are notes. A table that does not parse is `Inconclusive`. Every `STR#` is also compared as `sims-vm` loads it: its own parse, the language-1 entries in file order (`program.rs:615`) and by language runs (`loaded_order`, `program.rs:719`). When the two readings disagree about whether the tables match, in either direction, the result is `Inconclusive`. Primitives read these tables (Run Tree By Name #303, Change Suit, dialogs). |
| `BCON` | Values must match; the flags byte is a note. |
| `TPRP`, `TRCN` | Editor metadata, notes only. |
| Everything else | Bytes must match. |

Two more file problems make the verdict `Inconclusive`:

- A file whose resource map picks a different chunk for some `(type, id)`
  than `sims-vm`, which loads every chunk in file order so the last one wins
  (`program.rs:601`). Duplicate or deleted chunks cause this.
- Duplicate chunks of any type that are not the same, copy for copy, in both
  files. `sims-vm` parses every copy as it loads, so a malformed earlier copy
  stops the load even when a later one would be kept.

String tables are parsed even when their bytes match, so identical tables
that do not parse are `Inconclusive` too.

Other chunks' labels, and all chunk flags, are notes. Reports name resources
and entry indices only, never string contents.

The verdict is Inconclusive if a resource map is unusable (the compared chunks
may not be the ones the game loads). Otherwise it is Different if anything
differs, otherwise Inconclusive if anything is undecided, otherwise
Equivalent.

With `--tree`, the other trees are skipped but every other resource is still
compared, since the tree may read constants, strings and tables. Its callees
are assumed to behave the same; a whole-file run checks them as their own
resources. The CLI exits 0, 1 or 2 for
these, and with 2 on usage or I/O errors. The CLI flags are:

- `--tree <id>`
- `--with <iff>` (repeatable)
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
  it (`object_uses.rs:246`). This makes the Fridge tree differ from
  `return true;`, and a loop that returns from its failed exit test differ
  from straight-line code. `--ignore-last-result`
  (`LiveOptions::observe_last_result = false`) proves such trees equivalent.
  Use it when the tree is not the root of an action, as a check tree is.
- **Attributes 8 and up are opaque** unless `--my-attributes` says the
  running object has them. Reading them can trap (native 6).

## Results (Complete Collection, `SIMS_INSTALL`, in memory, counts only)

`tests/live_equiv_corpus.rs`, default options (last result observed):

- **Self-check:** 28,690 trees, each checked against itself.
  - **16,868 Equivalent.**
  - 11,822 Inconclusive. Of these, 11,423 have a loop whose exit depends on an
    event or input, 298 exceed 4,096 paths, 92 exceed the pair budget and 9
    use Global 34.
  - **0 counterexamples.**
- **Structured lift, recompiled:** of the 27,781 trees lockstep proves
  equivalent, live-output agrees on 16,794, is Inconclusive on 10,987 (the
  same loop limit), and **contradicts it on 0**.
- **One-node dummy compares** whose branches agree: 309 trees.
  - Last result observed: 46 proven, 172 refuted, all on the last result
    alone (0 on anything else).
  - Last result ignored: 218 proven equal to `return true;` or
    `return false;`, 0 refuted, and the rest read memory that can trap.
    **`Fridges.iff` tree 4143 is proven equal to `return true;`.**

## Limits, and what would reopen this

Path enumeration cannot summarize a loop whose exit depends on an event (such
as `while` around a call, or idle and retry), so these trees are
`Inconclusive` in live-output. The combined checker still proves them
whenever lockstep does. Closing them needs an event-aligned product
bisimulation with symbol renaming at loop heads, which would memoize
(pcA, pcB, canonical frame) after each pair of matched events. That is the
natural follow-up if a migration needs dummy-node elimination inside such
loops.
