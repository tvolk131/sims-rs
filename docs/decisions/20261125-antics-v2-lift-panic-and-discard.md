# `lift --v2` writes `panic!();` and `let _ = f();`

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/decompile.rs`; lift test expectations; new
  `v2_lift_panic_and_discard.rs`
- **Track:** Track 9 (`lift --v2`), `.antics` v2.1 Rust-Idiom Polish, Step 2
- **Builds on:** [`20261125-antics-v2-panic-and-discard-let.md`](20261125-antics-v2-panic-and-discard-let.md)
  (#475) and [`20261125-antics-v2-lift-rust-polish.md`](20261125-antics-v2-lift-rust-polish.md) (#476)

## Context

#475 gave the language two Rust spellings:
- `panic!()` for the error exit, which used to be written `return error;`;
- `let _ = expr;` for a value nothing reads.

The lift still wrote the old forms:
- `return error;`, which reads as returning a value named `error`;
- `if f() {}`, a test with nothing in it.

## Decision 1: the error exit is `panic!();`

- **The generator writes `panic!();`** wherever it wrote `return error;`. #475 parses it to the
  same statement, which lowers to the same `.air` and bytes. The one exception is a binding
  named `error`: `return error;` would return that binding, while `panic!()` is always the
  exit. So `panic!()` is the exact spelling. Animation event handlers still get no error
  exit, as before.
- **`raw { }` keeps `return error;`**, because that is `.air`'s own syntax.
- **`f()?;` still applies.** The lift's matcher for lint `prefer_question_mark`
  (`question_mark_end`) accepts either spelling, so it keeps working when the lint moves to
  `panic!()`.

## Decision 2: an unused call result is `let _ = f();`

- **What qualifies.** The statement `if f(..) {}` or `if !f(..) {}` becomes `let _ = f(..);`
  when all of these hold:
  - the test is a call alone (a path or method chain ending in its arguments);
  - the block is empty, with no comment;
  - there is no `else`;
  - it is not the `if` of an `else if`.
- **What doesn't.** A comparison (`if param[1] == 1 {}`) keeps its `if`: it is a test, not
  a call whose result is dropped.
- **Why `!` can go.** It only swaps the branches, and here both lead to the same place.
- **Byte gate.** #475 lowers `let _ = f();` as `if f() {}` for a bool or `Result` call,
  but as the statement `f();` for some builtins, and as a hidden local for a value. So
  each rewrite is kept only if the object builds the identical `.iff`. The rewrites are
  tried together, then in halves, with at most 8 builds per file (`Model::polish`).
  Calls that would change keep their `if`.
- **Order.** This pass runs before `else if` joining, so `else { let _ = f(); }` stays an
  `else` block rather than becoming `else if f() {}`.

## Results

RESULTS

## Consequences

- Lift test expectations that pinned `return error;` or `if f() {}` now expect the new forms.
- Lint wording that says "`return error`" (Track 7) may now point at a `panic!()`. The lift's
  own `?` matching already accepts both spellings.
