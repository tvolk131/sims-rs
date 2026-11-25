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
    cursor move (Batch 1 Stoves 4110 R3-6);
  - `push_interaction(object: <@slot let>)` was refused (CashRegisters 4098 gap 3);
  - `job_data(..) == k` went through a fresh `__job` local (Dressers 4117 bug 3).

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

**Global 34.** Every Expression node, the save included, sets the object's scratch pointer, and
a read of global 34 depends on it (`sims-vm` `primitives/expression.rs`). So a tree whose `.air`
may read global 34 (`may_read_global_34`) keeps every Expression main keeps:

- such a read is a `global[34]` operand, a `global[..]` whose index is not a literal, a
  `scope(6, ..)` cell, or a raw `op` (its opcode a number or a constant);
- a text tree takes `Unknown`, which is main's rule;
- out-operands are not definitions there (section 5), so they make no store dead;
- no pass drops its entry save: neither the optimizer (`drop_unread_entry_save_node`, which main
  runs in a tree that formats no text) nor the final checks in `finish_body` (the raw-layout path
  and the text path).

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
  field after a dot and the tree a raw `call` names do not count. A raw block may read `__self_obj` or `__saved_stack_obj` but may not declare a local,
  constant or label of either name (`check_raw_reserved_names`). A source constant named
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
    declaration, and never the cross-helper merge.
    - Lowering records each inlined local's declaration (`local_origins`: the helper and the
      innermost statement being lowered). That covers `let`, `let Some`, `for` and every other
      binding, so another helper's local of the same spelling is not a copy. The statement is
      restored when a nested body ends, so a `let Some` binding, which is allocated after its
      `else` body, still has the `let Some` as its declaration.
    - When any copy of a declaration is written through an out-operand, all its copies count as
      written. A specialization that drops the write from one copy keeps them one group.
    Bookcases 4107 shows why: Maxis keeps a helper's `event` and `pick` in separate words, and
    merging them made the verifier's cut-point mode compare unequal slots.
- **`self` before the first cursor move (`lower/stmt.rs` `lower_tree_body`).** In a Sim tree, the
  leading top-level statements before the first one that may move the cursor run with `stack_obj`
  still the entry object (`stmts_may_clobber_stack_obj`, which looks through called helpers). There
  two `self` sites use the stack object in place, with no switch and no staging local: a
  primitive's `self` target (`emit_on_self`), and a `self` cell compared with a compile-time
  constant (`self.<field>`, `self.attr[k]`, `self.data[k]`). Every other `self` read and every
  write is staged as on main. Both sites are marked by `self_in_place` while they lower.
  - Source order is control order only when no raw code defines a label or branches to a node by
    number. Otherwise a branch from after a move may jump back before it. Lowering itself emits
    neither, so the check reads the tree's finished `.air` (`raw_facts`). That covers raw code from
    every route: the tree's own blocks, helpers it calls, parameter defaults, and helpers named like
    statements. When a tree that used the prefix has such code, the object is lowered again with no
    prefix in any tree (`ObjectEnv::in_place_off`), as for the text fact.
  - The prefix also drops the Expression that staged a `self` read and its local. The `job_data`
    comparison below drops a copy the same way, so it is an in-place rewrite too: it sets the same
    flag (`in_place_applied`) and is off under `in_place_off`. So the same
    fallback applies when the finished `.air` may read global 34 (section 1), whose value depends
    on the scratch pointer that Expression set. It also applies when the tree's text may read a
    frame word, because the named locals after the dropped local move down a word.
  - An in-place read is staged and ordered exactly as main stages the switched one. The purity
    checks (which operands are staged and in what order, and whether a loop header is one node)
    ask whether `self` switches after a move (`self_switches_after_move`), not where it reads now.
    So the read stays inside a loop whose header reads it, and the operands main reads before it
    stay first. The prefix saves only the switch and the staging local where the read is the last
    operand read.
  - The VM resolves a comparison's right operand before its left. So a `self` read on the left of
    a comparison is staged first, as after a move, unless the right side is a compile-time
    constant (as for `job_data`). Otherwise a faulting right operand would fault before the `self`
    read.
  - The scratch pointer a tree leaves for its callees and callers is part of its lowered shape,
    which `verify-equiv` observes at every call and exit. It is not a rule of the source: main's
    entry-save drop in non-text trees already ignores it.
  - A raw block in an inlined helper or under a cleanup may branch only to its own labels. That
    check now covers raw `loop` bodies too.
  - A padded raw primitive (`… pad 0xNN`) counts as the primitive it wraps, both for moving the
    cursor and for formatting text.
- **`push_interaction(object: <@slot(N) handle>)` (`lower/builtins.rs`).** It emits `object: N`
  with flag 0x02, so the VM reads the handle from its own pinned word. There is no copy into word 0
  and no pin, so it can share a tree with other `@slot` lets. An explicit flag 0x02 is refused, as
  for any handle.
- **`job_data` comparisons (`lower/cond.rs`, `lower/expr.rs`).** `job_data(..) <op> k` reads the
  `scope(33, field)` cell directly in the comparison node when `k` is a compile-time constant.
  - The VM resolves the right operand first. So an operand that can fault, such as a temp or an
    attribute, would fault where the copy faulted on the career lookup.
  - Any other right side is still copied to a local.
  - Dropping the copy drops an Expression and a local, so the tree is checked after lowering like
    the `self` prefix (section 5), and the object is lowered again with the copy when the check
    fails.

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
    out-operand's unwritten exit, loops and raw back edges that move the cursor, and a faulting
    `job_data` comparand.
  - The `opt.rs` unit tests cover the pipeline's text veto.

## What would reopen this

- A Maxis layout that the out-operand coalescing restriction gets wrong the other way: distinct
  helper locals that Maxis does share.
- Text read from a file other than the compiled object's own, without `@slot`.
