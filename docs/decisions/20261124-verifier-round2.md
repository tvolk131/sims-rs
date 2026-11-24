# Verifier round 2: liveness, renaming, scratch pointer, data words, dead arguments

*Decided 2026-11-24.*

- **Scope:** `crates/sims-antics/src/verify/**` and the `verify-equiv` flags in
  `bin/sims-antics.rs`
- **Builds on:**
  - [`20261124-verifier-pilot-upgrades.md`](20261124-verifier-pilot-upgrades.md)
  - [`20261123-verifier-behavioral-mode.md`](20261123-verifier-behavioral-mode.md)
  - [`20261123-live-output-equivalence-verifier.md`](20261123-live-output-equivalence-verifier.md)

## Context

The second round of the multi-asset `.antics` pilot left trees INCONCLUSIVE for these
reasons:

- **last result:** a port writes a comparison another way, so the last branch result
  differs before a primitive or call that sets it again before anything reads it;
- **local slots:** a port numbers its `let`s another way, and Dialog text (`$Local:N`)
  pinned every slot;
- **scratch pointer:** a port moves an `Expression` across a primitive that cannot read
  the scratch pointer;
- **dead reads:** a port drops a read of a data word whose value is overwritten or never
  used;
- **dead arguments:** a port passes something else in an argument the callee overwrites
  before it reads it;
- **temporaries:** a port stages `temp[i]` again before each read.

As before, every verdict keeps its contract: EQUIVALENT is sound, DIFFERENT is a run
that differs, and everything else is INCONCLUSIVE. Each rule below has synthetic
adversarial tests in `tests/equiv_round2.rs`. `live_output_checker_is_sound_on_the_install`
still finds 0 counterexamples and 0 false equivalences.

## Decision 1: last result before a node that sets it first

A node sets the running object's last branch result only when its primitive returns a
branch (`interpreter.rs:1479`). A yield or a fault leaves it. The old value is then read
by:

- a resumed primitive;
- recovery (`people.rs:1297`, `visits.rs:280`);
- a person letting go of an object (`forget.rs:142`);
- Idle (17) or Gosub Found Action (30);
- a tree that some primitive runs on the running object's own stack and then pops.

`overwrites_last_result_first` lists the nodes that never yield, run no tree, and
cannot make `sims-vm` run one on the object:

- comparisons;
- `Expression` writes to a temp, parameter or local;
- data-only primitives;
- Generic Sims Call (1), except routines 19 and 20;
- Random Number, Get Direction To, Relationship and Set To Next, when their destination
  is plain.

A call qualifies too when the callee is a private, non-indexed tree of the same file
whose first node qualifies (`last_result_overwriters`). The call must come from a
private tree, because only private trees run the object's own 4096+ trees.

Before such a node, the last result is dead **except at its fault**. So the rule applies
only under `--ignore-fault-last-result`, which is already an ASSUME line.

**The default still observes the last result.** Each side records it unless its own node
hides it, so two values are equated only when both sides hide them.

**For Track 9 (lint):** `negated_comparison` and inverted-return rewrites change the last
result. In the default mode they are behavior-changing, and only provable with
`--ignore-fault-last-result`. Lint suggestions should say so.

**Found by a negative test, and fixed:** an opaque write event never recorded the last
result. A write to global 3 (co-op selection) may run PLACED on a person
(`memory.rs:846`). Global writes now record it.

## Decision 2: locals renamed past Dialog and Action String text

Each Dialog and Add/Change Action String names the strings it formats by operand:

- Dialog: buttons by type, then operands 6 and 2, and operand 1 in mode 4;
- Action String: operand 4 of table word 0.

For each, `v` names string `v - 1`, and 0 names none. So the locals the node may read are
the `$Local`/`$TimeLocal`/career tokens **of those strings** (`verify/text.rs`), not of
the whole table.

The string at an index is taken from every chunk a loader could read it from. For a
`STR#` chunk the loader reads as ported (`unported_strings` empty, and some US English
entry exists), both orders `sims-vm` uses count:

- file order (`Library::strings`);
- the loader's order (`loaded_order`, where a later language run goes in front).

Any other `STR#` chunk, every `CTSS` chunk, and every legacy `CST` chunk count at every
index. A string with any of these may read any local:

- `$Me`, `$Object`, `$Neighbor` or `$Family`: a name may hold a token;
- a token of an unported pass;
- a `$Local` right after `:` or a digit.

The renaming keeps the slot numbers those strings name. It also now renames:

- a local a primitive reads **by value** (Alter Budget's amount, and so on): the event
  records the value, not the number;
- Random Number's destination: it always stores its draw (`random.rs`). The draw is a
  fresh value keyed by its event (`random@eventN`), so a copy of it is the same on both
  sides.

## Decision 3: the scratch pointer is observed only where it can be read

The scratch pointer has only two readers in `sims-vm`:

- an `Expression` with a Global 34 right operand (`expression.rs:34`);
- UI Effect reading Global 34 (`ui_effect.rs:371`).

The checker refuses Global 34. So a primitive observes the state only if it may run code
that reads it.

These primitives return a bool, never yield, and run no tree, so they record no scratch
state (`scratch_effect`):

- Refresh 7, Own Events 10, Get Distance To 11, Breakpoint 15, Show String 21;
- Play Sound 23, Old Relationship 24, Alter Budget 25, Set Motive Change 29;
- Test Object Type 32, Find Worst Motives 33, Sim Interacting With 37,
  Stop All Sounds 48.

Random 8 and Set To Next 31 set the state before anything else, including any fault
(`random.rs:22`, `set_to_next.rs:116`). Get Direction To 12 (`direction.rs:49`) does the
same only when its destination scope word fits a byte. It checks that word first
(`direction.rs:45`), so with a larger one it fails and leaves the state as it was; then
it counts as observing. Sol's first review found this. Read events do not record the
state either.

**Push Interaction (13) is not blind.** It may run an at-once interaction's trees
synchronously as this object (`in_way.rs:674`). Neither is any call.

The state stays compared at every exit and at every fault: the object keeps it, and its
next Global 34 read sees it. The new `--ignore-fault-scratch-pointer` drops it at faults
only, as an ASSUME line, like `--ignore-fault-last-result`.

## Decision 4: the running object's data words are plain values

Every object has at least 72 data words:

- 72 from the factory (`factory.rs:325`) and for saved objects (`saved_objects.rs:385`,
  `objects.rs:579`);
- 128 for co-op and placement.

Scope 3 bounds its index by 72 (`memory.rs`, `bounded(index, 72, 7)`). So a read of
`my.data[0..72]` never faults.

Only events change the running object's data, since writes to scope 3 are opaque write
events. A read is therefore a value keyed by the last event (`my.data[i]@eventN`), not an
event. Two reads with no event between them are equal, and a dropped dead read is no
difference.

Word 25 (LOCKOUT) stays a read event, because reading it wakes the object
(`memory.rs:137`). Indexes 72 and up, and negative ones, also stay events.

**Domain.** The 72-word floor holds for every object the game builds. Every builder
outside test code gives 72 or more, and nothing shortens `data`. `World::decode` does
not check it, though: a snapshot written by hand with a 64-word object would fault on a
dropped read of word 71. Such a snapshot is not a state a run of the game reaches, so
the rule stands. Sol's second review found this. Follow-up for the snapshot owner:
`validate_world_object_ids` (`boot/save.rs`) could reject objects with fewer than 72
data words.

## Decision 5: dead call arguments

A call from a private tree to a private, non-indexed tree of the same file, with the
callee's arity known (at most 4: the operand words, or the temps for all-0xFF operands,
`interpreter.rs:1539`), compares only the arguments the callee may see
(`FrameVal::CallArgs`). The context must also agree with the loader's arity.

`params_written_first` finds the parameters the callee overwrites before it can see
them. It follows the callee's chain of assignments from node 0:

- In both relations, the chain may assign from a literal, a temp or a parameter already
  written.
- In the behavioral relation, the chain may also assign from a read that may fault. The
  behavioral relation sees no frame at a fault, and strict mode does.
- In the behavioral relation, a parameter no node reads at all also counts
  (`params_never_read`). Reads include Expression right operands, left operands of
  non-assignments, scope 22 on either side, and every slot `primitive_frame_uses` lists.
  An unmodeled primitive reads the whole frame.

Both sides use the **intersection** of their two files' sets for each callee. Any part
of a side's set is still dead on that side, and the two sides' call events must leave out
the same arguments. Using each side's own set made Stoves 4124 regress: the ported
callee reads no argument, and the original's reads one.

Strict mode still compares a faulting call's raw bytes, so in practice the rule proves
equivalence only behaviorally.

**Given trees must be the file's.** `check_tree_with_callees` (`equiv`) takes each
side's trees and arities next to its file. Dead arguments and last-result overwriters
come from those trees, so a description that differs from the file could prove a pair
the VM tells apart. Sol's second review found this. Now each given private tree must be
the file's kept tree, byte for byte, and each given private arity must be the file's,
or the check is INCONCLUSIVE. A tree or arity left out stays unknown, as before
(`trees_given_with_a_file_must_be_the_files`).

**Stale frame words.** A dropped argument stays in frame storage after the callee
returns (`interpreter.rs:1740`). A later frame at the same depth starts with it as an
uninitialized local (`interpreter.rs:1218`). The behavioral relation already assumes
stale frame words agree on both sides (see "What a behavioral proof does not cover" in
the behavioral-mode decision), so dead arguments add another source of such a word, not
a new kind of gap. Strict mode observes frame storage and keeps the argument
(`a_dropped_argument_a_later_frame_may_read_stays_strict`). Sol's second review found
the pair.

## Decision 6: temporaries staged again need nothing new

Staging `temp[i] := param[j]` again before each read writes the same symbolic value. The
checker already compares values at events, not writes (see the
`temporaries_staged_again_are_the_same_values` test).

The two Computers 4097 experiments that looked like a staging gap are **real port
differences**. The dialog's `$Local:0..4` read other job-data words in the port: the
optimizer coalesced `let`s read only by `$Local`, and the un-optimized build laid the
slots out differently. The shipped Computers port is EQUIVALENT.

## Not resolved

- **Pool 4105:** a real port difference. The original's call to global 0x0127 at n51 goes
  to error on false, and the port's n65 goes on.
- **CashRegisters 4098:** the scratch state differs before Push Interaction, which may
  run an at-once tree as this object. Not provable soundly.
- **Piano 4110:** needs a non-injective renaming. The original reuses local 0 for two
  Animate events, and the port uses two locals.
- **Stoves 4134:** frame shape, 0 locals against 1 (the Pool 4100 follow-up).

## Benchmark

The benchmark is the pilot's local benchmark: 55 runs, 42 behavioral and 13 strict,
with no assumption flags. Before is a9eacd99 and after is this change.

| | before | after |
|---|---|---|
| EQUIVALENT runs | 8 | 17 |
| trees left unproven, summed over runs | 347 | 338 |
| trees proven before and lost | | 0 |

These trees are newly proven:

- Beds 4162 to 4165 (dead arguments);
- ChessTable 4103 and Mailbox 4103 (renaming);
- Computers 4132, which makes the whole Computers port EQUIVALENT (per-string text);
- Toilets 4129 (a pure data-word read).

With `--ignore-fault-last-result --ignore-fault-scratch-pointer`, three more are proven:
ChessTable flip 4098, Fridges flip 4116 and Stoves 4105.

## Consequences

- `--ignore-fault-last-result` now proves more: polarity rewrites before nodes that set
  the result first.
- `--ignore-fault-scratch-pointer` is new; when set, the run prints it as an ASSUME line.
- Dialog-heavy objects rename locals as long as the strings they show name none.
- A tree reading only its own data words no longer needs those reads in step.
