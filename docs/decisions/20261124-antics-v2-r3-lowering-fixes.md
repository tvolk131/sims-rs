# `.antics` v2 Lowering: Round 3 Fixes

*Decided 2026-11-24.*

**Branch:** `t3/antics-v2-r3-fixes`  
**Track:** Track 8 (`.antics` v2)

## Context

The Round 3 pilots (13 objects, Batches 1–4 plus Fridges) found these lowering problems on main
(`14f25c58`):

- **Dialog trees kept unread entry saves (R3-1).** A Sim tree that formats text (`dialog`,
  `action_string`) kept its unread `let __self_obj = stack_obj;`, even when no string of the object
  can read a frame word, or when every named local is `@slot`-pinned. Affected: Chess
  4106/4113/4114, Piano 4097/4103, Fridges 4112, Dressers 4126 and Bookcases 4119.
  - Related (2b): a `random_into` whose range is a BCON constant was not counted as writing its
    destination.
- **Hidden `local: 0` words (Batch 4 bug 2).** The report said builtins that emit a `local: 0`
  operand reserve frame word 0.
- **`@context(inherit)` false positives (Batch 1 R3-2, Batch 2 D-2).** The check compared the two
  lowered texts literally, so a body that differed only in generated local names failed. The error
  also pointed at the tree instead of at the statement that differs.
- **Rough edges:**
  - disjoint inlined helper copies did not share a word when `animate(local:)` or `random_into`
    wrote their local (Batch 2 Piano 4110 R3-2);
  - a Sim tree staged `self.*` reads through a `__pre_stack_obj` local even before its first
    cursor move (Batch 1 Stoves 4110 R3-6; kept, see section 5);
  - `push_interaction(object: <@slot let>)` was refused (CashRegisters 4098 gap 3);
  - `job_data(..) == k` went through a fresh `__job` local (Dressers 4117 bug 3; kept, see
    section 5).

## Decision

The code is the definition of each rule below. This section says what changed and why.

### 1. Which frame words the object's text may read (`mir/text_reads.rs`)

A compiled object's file holds exactly the strings its source gives it:

- its `strings` tables;
- the `STR#` 301 entries its inline dialog literals add;
- its catalog strings (names and descriptions, variants included).

The text primitives format only strings of the running tree's own file. So the frame words any of
the object's text can read is the union of what the verifier's token rules
(`verify::string_locals`) find in those strings. That union is the new object-level fact
`ObjectEnv::text_reads`, a `TableLocals`:

- `Known(∅)`: no string names a word;
- `Known(S)`: the strings name at most the words in `S`;
- `Unknown`: some string has a token whose reads cannot be bounded, such as `$Me`.

Inline dialog literals are interned while trees are lowered. If a later tree adds a literal that
changes the fact, the object is lowered again with the final fact (`lower_object_with`).

Each tree's `FrameFacts::text_reads` is the object's fact if the tree formats text, and `Known(∅)`
if it does not:

- **`Known(∅)`**: a text primitive is just a primitive. The entry save may drop, and the H-3 rules
  for text trees are lifted: source locals keeping their stores and words, no cross-helper sharing,
  and no `let` in a `const` binding's region. Any other result keeps those rules.
- **`Known(S)`**: the unread save is dropped only if every word in `S` keeps the same occupant
  with and without the save. `frame_occupancy` computes the occupants the way `.air` reads the
  placed body:
  - an `@slot` local that the body names only in its `let x;` holds no word;
  - the floor is the `min_locals` the header actually carries, and `@frame(locals: K)` gives none.

  The save's own word always changes hands. So does the word after the named locals, where `.air`
  keeps `param[0]` around an `idle` when the tree uses both. The slot scan does not report that, so
  the word is always counted as taken. So this drops the save when every named local is
  `@slot`-pinned or the save is the highest word, and keeps it whenever `$Local:N` could read a
  word that would move.
- **`Unknown`**: the save stays.

A tree that may run as an object's bottom frame takes `Unknown` when it formats text
(section 5, "Bottom frames").

**Global 34.** Every Expression node, the save included, sets the object's scratch pointer, and
a read of global 34 depends on it (`sims-vm` `primitives/expression.rs`). So a tree whose `.air`
may read global 34 (`may_read_global_34`) keeps or drops an Expression exactly where main does.
Keeping one main drops is no safer than the reverse: a read of global 34 before any other
Expression faults on main and succeeds after a kept save. So such a tree takes main's rules:

- such a read is a `global[34]` operand, a `global[..]` whose index is not a literal, a
  `scope(6, ..)` cell, or a raw `op` (its opcode a number or a constant);
- a text tree takes `Unknown`, which is main's rule;
- out-operands are not definitions there (section 5), so they make no store dead;
- a `random` defines its destination only with a literal range (`DrawRanges::Literal`, main's
  rule from section 2), wherever that decides whether the entry save goes;
- the entry save goes where main drops it, in the optimizer (`drop_unread_entry_save_node`) and
  the raw-layout path, and the text checks keep it as main's text rule does;
- the pinned slot pins as on main, as it does in every tree (section 5);
- out-operand locals coalesce as on main (no `decl:` groups), so the tree fits the frame main
  fits it in.

The rule is a permission, not an obligation. As on main, dead-store elimination drops an unread
save whose next node ignores the last result. It does this whatever the text reads, because the
save is a compiler temporary, not a source local. Under the `$Local:N` contract (PR H) only
`@slot(N)` fixes a word's number.

Text in another file (a host file a compiled tree is spliced into) is outside what the compiler can
see. Under the same contract, `@slot(N)` is the way to fix the words such text reads.

One string outside the object is formatted too: a dialog button with no label string gets the
game's default label (`STR#` 142 of `GameData/UIText.iff`). The fact assumes those labels hold no
token. `sims-vm` makes the same assumption, checks it against the install
(`labels_hold_no_token`) and fails loudly when it does not hold.

### 2. `random` ranges (2b, `mir/frame.rs`)

The VM resolves a Random Number range read-only. The only cell that then yields a word pointer, and
so takes the draw in place of the destination, is global 34 (`sims-vm` `memory.rs` `resolve`).
`random` now counts as writing its destination on both exits unless the range is spelled with
`global` or `scope` (`range_may_take_draw`). This supersedes PR H's "only when its range is a
literal": constants (`bcon[..]`, named constants), locals, temps, parameters and attributes all
leave the draw in the destination.

### 3. Hidden `local: 0` words (Batch 4 bug 2): no change

The `.air` compiler already reserves a primitive's local byte only when the VM reads it:

- `relationship` for directions 2 and 3;
- `create_object` for modes 8 and 9;
- `make_new_character` always (skin, age and sex).

`create_object(.., mode: off_lot)` and `relationship` in directions 0 and 1 cost no word
(`tests/v2_r3_fixes.rs`). The MailBox trees the report lists (4102, 4117, 4119, 4126) use
`relationship(dir: 2 | 3)` without `local:` and a raw `make_new_character`. The VM genuinely reads
word 0 in both, and Maxis keeps the person or the mailbox there. The fix belongs in the source:

- `relationship(.., local: x)` with an `i16` let, which is already supported;
- or `@slot(0) let …`.

### 4. `@context(inherit)` check (`lower/trees.rs`)

The two passes are compared after canonical renaming (`canonical_locals`): `let`/`local` names in
declaration order, so generated names no longer matter.

- The Sim pass declares its implicit entry object (`__self_obj`) when the body reads it, as the
  object pass declares `__saved_stack_obj`. A read is an `.air` token naming it, so a comment, a
  field after a dot and the tree a raw `call x`, `subroutine x` or `subroutine(x)` names do not
  count. A raw
  block may read `__self_obj` or `__saved_stack_obj`. It may not declare a local, constant or
  label of either name in a tree whose lowering declares that name too
  (`raw_reserved_declarations`, checked in `finish`). Elsewhere, as on main, it may. Code a
  constant condition discards declares nothing. A raw block's own local, constant or label named
  `__self_obj` is not the entry object either, so an inherited tree with one gets no entry save
  for it, as on main. A source
  constant named
  `__self_obj` is a constant to the raw blocks that read it, not the entry object, and so is a
  qualified constant such as `__self_obj.tuning`, which a raw block may declare.
- A tree whose two lowerings differ still fails. The error points at the first source statement of
  the tree whose lowering differs (`differing_statement`, via the statement marks `TreeLowerer`
  records), such as a targetless `play_sound`.

### 5. Rough edges

- **Out-operands define locals (`mir/liveness.rs`).** Liveness now treats a primitive's out-operand
  write as a definition on the exits that write it, through the shared `out_operand_write`. A tree
  that may read global 34 keeps main's rule (`Liveness::out_defs`), as in section 1. For
  example, `animate(local:)` writes on its false exit, so in `if animate(..) { break; }` the
  `break` arm keeps the old value live and the fall-through arm does not.
  - Disjoint inlined copies of a helper now share their local's word.
  - In `coalesce_locals`, a local written through an out-operand joins only classes of its own
    declaration, and never the cross-helper merge. A tree that may read global 34 (section 1)
    or bounds its frame, with `@frame(locals: K)` or with a `@frame(format: F)` (formats
    `0x8000`/`0x8001` hold four locals), coalesces them as main does, so it fits the frame main
    fits it in.
    - Lowering records each inlined local's declaration (`local_origins`: the helper and the
      innermost statement being lowered). That covers `let`, `let Some`, `for` and every other
      binding, so another helper's local of the same spelling is not a copy. The statement is
      restored when a nested body ends, so a `let Some` binding, which is allocated after its
      `else` body, still has the `let Some` as its declaration.
    - When any copy of a declaration is written through an out-operand, all its copies count as
      written. A specialization that drops the write from one copy keeps them one group.
    Bookcases 4107 shows why: Maxis keeps a helper's `event` and `pick` in separate words, and
    merging them made the verifier's cut-point mode compare unequal slots.
- **`self` before the first cursor move: not changed (Stoves 4110 R3-6).** A Sim tree that moves
  the cursor still selects `self` through `__self_obj`, with main's staging, even before the first
  move. An in-place read was tried and dropped. The staging is a pair of Expressions, and each
  leaves the last result true. A read or primitive that faults after them (an index out of range,
  a missing object) leaves that true behind, and an in-place one leaves the previous result.
  `verify-equiv` observes the last result at a fault by default: play-mode recovery keeps it in
  the rebuilt frame and copies it into a reset person's last-action flags. No safe subset was
  found: every stack-object read faults when the object is missing. The 13 pilots are EQUIVALENT
  without it, so the shape costs no verdict.
  - The scratch pointer a tree leaves for its callees and callers is part of its lowered shape,
    which `verify-equiv` observes at every call and exit. It is not a rule of the source: main's
    entry-save drop in non-text trees already ignores it.
  - A raw block in an inlined helper or under a cleanup may branch only to its own labels. That
    check now covers raw `loop` bodies too.
  - A padded raw primitive (`… pad 0xNN`) is still not read as the primitive it wraps, as on
    main, either for moving the cursor or for formatting text. Reading it that way changes which
    trees get an entry save and so moves the frame words their text reads; it is left for a
    change of its own.
- **The pinned `push_interaction` slot (`mir/pipeline.rs`).** `pin_in_place` makes a pushed
  handle live in word 0 and drops its copies, which moves every named local down a word and fills
  word 0 from the start. A primitive may read word 0 without naming it (`relationship` without
  `local:` reads the word-0 pair), and the pin scan does not see such reads, so filling word 0
  early can change what one reads. The tree therefore pins exactly where main's literal-range
  analysis pins (`DrawRanges::Literal`), never on section 2's wider range rule, whatever its text
  reads. Before the body is final, the slot
  is a placeholder (`@pin@`) that the lexer does not read. Global 34 is checked on the body with
  word 0 in its place, so a pinned text tree is no longer `Unknown` for that reason alone.
- **`push_interaction(object: <@slot(N) handle>)` (`lower/builtins.rs`).** It emits `object: N`
  with flag 0x02, so the VM reads the handle from its own pinned word. There is no copy into word 0
  and no pin, so it can share a tree with other `@slot` lets. An explicit flag 0x02 is refused, as
  for any handle.
- **`job_data` comparisons: not changed (Dressers 4117 bug 3).** `job_data(..) <op> k` still
  copies the career cell into a `__job` local. Reading the cell in the comparison was tried and
  dropped. It removes a frame word, and a frame's size is observable: Create Object with flag
  0x10 passes `temp[0]` to the new object's `main` only when that frame has a word for it, so a
  `main` that loses its only word faults there. It also removed an Expression and a node, which
  needed fallbacks for raw labels, positional targets, global 34, text and residue reads. The 13
  pilots are EQUIVALENT without it.
- **The fault state of dropped stores.** Main's dead-store elimination and its entry-save drop
  already remove Expressions before a node that may fault: a store a later plain write kills, and
  the unread save of a tree that formats no text. The last result and scratch pointer such a
  fault leaves then differ from the unoptimized tree's. This branch applies the same rules to
  more trees, namely a store an out-operand kills (above), the save of a text tree that reads no
  moved word (section 1). It adds no rule of its own
  that removes an Expression. A tree that may read global 34 is the exception to applying them
  more widely: there the branch keeps and drops exactly what main does (section 1).
- **Bottom frames.** Create Object with flag 0x10 (or 0x02) writes word 0 of the new object's
  bottom frame, and faults when the frame has none. So in a tree that may run as one, which
  local lives in which word is observable: a local read before it is written reads the
  caller's `temp[0]` when it lives in word 0. A tree that formats text and may run as a bottom
  frame takes main's text rule (`Unknown`): it keeps its declarations and entry save as main
  does. This covers an object-self tree with no arguments (an object's or a variant's `main`),
  and the inherit check's Sim pass over one, so both passes agree. The section 1 text rules apply
  to the others. Main's own passes still drop declarations in a tree that formats no text, as
  before. (Main's dead-store elimination can still drop a text tree's unread entry save, as
  section 1 says.)

## Verification

- **Round 3 pilots.** The 13 pilots, spliced into the original files, are whole-file EQUIVALENT with
  `verify-equiv --behavioral --all`.
- **R2 dialog-save repros.**
  - Chess is EQUIVALENT (main: modulo 4106/4113/4114).
  - Fridges round 5 is modulo {4112}. The save difference is gone. What is left is round 5's
    unpinned `guest` word, which round 6 rewrote and which is EQUIVALENT.
  - Piano R2 is modulo {4110}, the separate over-2048-instruction path.
- **`open-pack`.** `verify` reports 0 violations, and all 20 objects build byte-identical to main.
- **Tests.**
  - `tests/v2_r3_fixes.rs` (synthetic) covers each fix and its negative cases: kept saves (including
    the `idle` scratch word), a later literal reaching earlier trees, read-first helper locals, an
    out-operand's unwritten exit, the kept `self` switch and `job_data` copy, raw blocks' own
    entry-object names (including discarded code and inherited trees), main's
    choices in trees that may read global 34 or bound their frame by a format, and the pinned
    slot pinning only where main pins.
  - The `opt.rs` unit tests cover the pipeline's text veto.

## What would reopen this

- A Maxis layout that the out-operand coalescing restriction gets wrong the other way: distinct
  helper locals that Maxis does share.
- Text read from a file other than the compiled object's own, without `@slot`.
