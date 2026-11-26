# `.antics` v2.3 step 2 lint: `if let`, loop-handle flag words, unread `extern bcon` entries

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`; tests appended to
  `tests/integration/v2_2_step2_lint.rs`; one pinned assertion flipped in
  `tests/integration/v2_2_bitflags_and_return_error.rs`; `docs/roadmap.md` (Batch 21),
  `docs/legacy-mining-audit.md`, `docs/decisions/20261124-antics-test-runner.md`.
  No change to lowering, the parser or `v2/decompile.rs`.
- **Track:** Track 7 (lint), `.antics` v2.3 Step 2
- **Builds on:**
  [`20261126-antics-v2-3-step1-if-let-and-469-fixes.md`](20261126-antics-v2-3-step1-if-let-and-469-fixes.md)
  (#489), [`20261126-antics-v2-2-step2-lint.md`](20261126-antics-v2-2-step2-lint.md) (#483)

## Decision 1: `if let Some(v) = f()` binds `v` in its `then` block alone

#489 gave the lint only the arms it needed to compile: `stmt_exprs` sees the test call and
`stmt_blocks` both blocks. Every rule that tracks names now scopes the binding as lowering
does.
- **Names in scope** (`bound`, which hides `me`, `self`, Sim names and engine words): `v` is
  bound for the `then` block and dropped after it. It never leaks past the statement, unlike
  a `let`.
- **`unused_decl`:** the binding is judged against its `then` block only. A raw block
  reading its frame word (`__v_v_0`) reads it, for every `let` too. The help names `_v`, or
  `_v2`, `_v3`, .. when the block already names `_v` (an outer `_v` it reads would be
  captured). It does not offer the plain
  `if f(..) { .. }` test: that test is the call's own result, not "an answer came", so it
  may build other bytes or not build at all.
- **`uninitialized_local_read`:** an `if let` that rebinds the tracked name reads it in its
  test and its `else` block, but not in its `then` block, and assigns nothing.
- **`unreachable_code`:** an `if let` with an `else` whose two blocks both leave ends its
  block, as an `if` does. The note says "every branch of the `if let`". A test pins that the
  build drops the statement after it, as the help says.
- **`collapsible_else_if`:** `else { if let .. }` collapses, and so does an `if let`'s own
  `else { if .. }` / `else { if let .. }`. The token check (`if_chain_end`) already read
  `let Some(v) = f()` as a condition; each collapse is pinned to build the same bytes.
- **`stack_obj_clobber_after_self`:** a binding of the same name as a saved `self` handle
  hides the handle in the `then` block, so `stack_obj = v;` there is no restore. An
  `if let Some(self) = f()` block is not judged at all: whether a binding may be named
  `self`, and what `self.x` then reads, is an open language question (#489).
- **Trial builds:** an `if let` statement binds names, so a typed-field proposal in its test
  is built apart from the body's others (`Pending::flows`).
- **`collapsible_if_chain`:** a chain whose head is an `if let` is not reported, as a chain
  whose head is any test other than an equality is not: the rule judges the chain from its
  head, and an `else if x == ..` run after another kind of head is left as written.
- **Left as is:** a `let x` before an `if let Some(x)` counts the inner binding's reads as
  its own, as it already does for a shadowing `let x` in a nested block: a missed
  `unused_decl`, never a false one. `uninitialized_local_read` takes an assignment in one
  block of an `if let` (or in a nested block, even to a shadowing `let` of the name) as
  assigning the name after the statement, as it already does for an `if`: missed findings,
  never false ones. `unbounded_loop_without_yield` needs nothing new: the test is
  a call to a tree, which may yield.

## Decision 2: flag words on `lot.objects()` handles

#489 lowered flag methods on a `for o in lot.objects()` handle through the direct cursor, so
they build the operator's bytes. The lint now tracks such handles (`loop_handles`, a subset of
`bound` kept in step with it) and lets `prefer_method_syntax` and `untyped_engine_constant`
treat their named flag words (`o.flags`, `o.flags2`, `o.wall_placement`) as engine words, as
on `stack_obj`.
- A handle of an untyped object loop names no object type, so no state of the file can take
  the name. A BCON table of the handle's name wins, since `o.wall_placement` then reads that
  table's entry: no suggestion is made. A handle of any other kind (a `let`, a parameter, a typed loop) is still skipped:
  its object may have a state of the name that this file does not show.
- A `let`, `if let`, loop or animation event binding of the same name hides the handle; the
  name is not a handle after its loop.
- Both rules propose, so every suggestion is still kept only when it builds the same bytes.
- The lift corpora hold no flag operator on a loop handle, so neither corpus changes.

## Decision 3: unread `extern bcon` entries

`unused_decl` reports an `extern bcon` entry that nothing reads (roadmap item since #478).
An entry is read when:
- its name is read anywhere in the file set: a `Table::entry` path (any table name, since a
  `use` may rename the table), a `.entry` member, or a word of `raw` code; or
- a `raw` block reads its address: `bcon[N]` / `const[N]` (decimal or hex),
  `bcon[T:E]` / `const[T,E]` (address `T << 7 | E`) or `scope(26, N)`; or reads BCON at an
  index that is not a literal, or in a scope that is not one (then no entry of the set is
  reported). A literal starts with a digit: `_16896` is a block constant's name. Text in
  quotes (a tree label) and comments read nothing.

Each entry is judged alone: the table is the library's, so the private-table rule (judge a
table only once one entry is read, since a global tree may read it by index) does not apply,
and `extern mod` does not turn the check off. A `pub` block is not judged, since a file
outside the set may read it. A `_` name opts out. A file with an object built `from` a template
its own file does not declare is not judged either: the template's trees may read the
entries, and a template of the same name elsewhere in the linted set may not be the one the
file's `use` imports.

**The raw-address read counts on purpose.** The behavioral and strict lift corpora (1,191
objects each; 735 files declare 2,315 entries) declare every entry their trees read, by path
or as a raw `bcon[N]`. In strict mode most reads are raw. Dropping an entry read only by
number builds the same bytes (checked on a strict file), so strictly its name is unused, but
the lift writes it to name the raw read. Without this rule the check reported 451 entries
(behavioral) and 2,025 (strict), all of them lift output. With it, both corpora give 0
findings, and no other rule's count changes (behavioral 263, strict 6, as before).

## Consequences

- New tests in `v2_2_step2_lint.rs`: binding scope, unused bindings, unreachable code,
  collapses, clobbered handles, loop-handle methods and names, `extern bcon` reads.
- The #482 assertion that pinned "no method suggestion on a loop handle" now expects
  `o.wall_placement.set(..)`, built to the same bytes.
- **Follow-ups:** a rule suggesting `if let` for `if f() { .. temp[N] .. }` (it waits on the
  lift writing `if let`, Track 9); the outer-`let` reads hidden by an `if let` of the same
  name (shared with nested `let` shadowing).
