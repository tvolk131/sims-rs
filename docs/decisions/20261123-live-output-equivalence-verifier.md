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

## Decision

### 1. Live-output checker (`verify/live.rs`, `check_bhav_live_equivalent`)

Each tree runs on its own over symbolic inputs. Each path becomes a summary of
what the rest of the game can observe:

- **Exit:** true, false, error, a modeled VM trap with its native code, or a
  jump to a missing node.
- **Event trace:** primitives and calls with canonical operands; reads of
  memory that may trap or has side effects; writes to memory the checker
  does not track. Each event carries a snapshot of what it may read:
  - the changed tracked cells,
  - the scratch-pointer state,
  - the frame slots the primitive reads. This uses the same per-opcode table
    as lockstep, now factored out as `primitive_frame_uses`.
- **Final values of the tracked cells.** These are `temp[0..8]` and
  `my.attr[0..N]`. Every object has at least 8 attributes
  (`sims-vm/src/factory.rs:314`), so N defaults to 8, and reads and writes of
  these cells never trap and have no side effects.
- **Scratch-pointer state at exit.** Every `Expression` sets it
  (`expression.rs:51`), and a caller's later read of Global 34 sees it.
- **Frame residue (opt-in):** the frame's final params and locals.

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

- the joint condition constrains independent inputs only by literal bounds;
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

Global 34 operands, and Random or GetDirectionTo with Global 34 or scope 22
aliasing, return `Inconclusive`. Kind-4 trees are checked from every entry
node.

### 2. Combined checker (`check_bhav_equivalent_combined`)

The combined checker runs lockstep first and falls back to live-output. It
reports which method decided (`EquivMethod`). A lockstep counterexample is a
divergence in the stricter all-accesses-observable trace. When live-output
cannot decide, the result is `Inconclusive`, with the lockstep note attached,
rather than "different". `sims-antics equiv` now uses this checker.
`lift --verify-equiv` still uses lockstep alone, because a structured lift
should match node for node.

### 3. Whole-file checker (`check_iff_equivalent`, `sims-antics verify-equiv`)

Resources are matched by `(type, id)` the way the original looks them up
(`Iff::get`). Deleted chunks and `rsmp` are skipped.

| Resource | How it is compared |
|---|---|
| `BHAV` | Combined checker. Each side uses its own file's arities and BCONs, plus any `--with <file.iff>` arities. An argument-count mismatch is a difference. Locals, format, revision and trailing bytes are notes. |
| `STR#`, `CTSS`, `TTAs` | Parsed with the chunk's byte-order flag. Text and language must match; layout version and comments are notes. |
| `BCON` | Values must match; the flags byte is a note. |
| `TPRP`, `TRCN` | Editor metadata, notes only. |
| Everything else | Bytes must match. |

Chunk labels and flags are notes. Reports name resources and entry indices
only, never string contents.

The verdict is Different if anything differs, otherwise Inconclusive if
anything is undecided, otherwise Equivalent. The CLI exits 0, 1 or 2 for
these, and with 2 on usage or I/O errors. The CLI flags are:

- `--tree <id>`
- `--with <iff>` (repeatable)
- `--my-attributes <n>` (at least 8)
- `--ignore-scratch-pointer`
- `--strict-frame-residue`
- `--all`

## Policies and why

- **Frame residue is not observed by default.** A frame's final locals do
  become the next same-depth frame's initial locals (`interpreter.rs:1195`).
  Scripts that read a local before writing it are rare, and lockstep ignores
  residue too. `--strict-frame-residue` turns it on.
- **The scratch pointer is observed by default.** A dummy `Expression`
  placed before a primitive changes what a later Global 34 read sees. The
  Fridge tree and `return true;` both end in one `Expression`, so they agree.
  `--ignore-scratch-pointer` relaxes this.
- **Attributes 8 and up are opaque** unless `--my-attributes` says the
  running object has them. Reading them can trap (native 6).

## Results (Complete Collection, `SIMS_INSTALL`, in memory, counts only)

`tests/live_equiv_corpus.rs`:

- **Self-check:** 28,690 trees, each checked against itself.
  - **16,862 Equivalent.**
  - 11,828 Inconclusive. Of these, 11,419 have a loop whose exit depends on an
    event or input, 302 exceed 4,096 paths, 98 exceed the pair budget and 9
    use Global 34.
  - **0 counterexamples.**
- **Structured lift, recompiled:** of the 27,781 trees lockstep proves
  equivalent, live-output agrees on 16,793, is Inconclusive on 10,988 (the
  same loop limit), and **contradicts it on 0**.
- **One-node dummy compares** whose branches agree: 309 trees. 218 are proven
  equal to `return true;` or `return false;`, 0 are refuted, and the rest
  read memory that can trap. **`Fridges.iff` tree 4143 is proven equal to
  `return true;`.**

## Limits, and what would reopen this

Path enumeration cannot summarize a loop whose exit depends on an event (such
as `while` around a call, or idle and retry), so these trees are
`Inconclusive` in live-output. The combined checker still proves them
whenever lockstep does. Closing them needs an event-aligned product
bisimulation with symbol renaming at loop heads, which would memoize
(pcA, pcB, canonical frame) after each pair of matched events. That is the
natural follow-up if a migration needs dummy-node elimination inside such
loops.
