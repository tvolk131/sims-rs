# `.antics` v2.2 step 2b lint: bitflags list methods never yield; the step 2a words

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`; new `tests/integration/v2_2_step2_lint.rs`;
  `docs/roadmap.md` (Batch 20).
  No change to `v2/decompile.rs` or lowering.
- **Track:** Track 7 (lint), `.antics` v2.2 polish wave, Step 2b
- **Builds on:** [`20261125-antics-v2-2-temps-and-flags.md`](20261125-antics-v2-2-temps-and-flags.md)
  (#480, the list methods),
  [`20261125-antics-v2-2-step2-bitflags-and-retire-return-error.md`](20261125-antics-v2-2-step2-bitflags-and-retire-return-error.md)
  (#482), [`20261125-antics-v2-2-lint-polish.md`](20261125-antics-v2-2-lint-polish.md) (#479)

## Decision 1: every bitflags method is non-yielding

`NON_YIELDING_METHODS` gains `intersects` and the four list methods: `set_all`, `clear_all`,
`contains_all` and `contains_any`.
- Lowering folds a list into `A | B` and reuses the `set` / `clear` / `contains` /
  `intersects` lowering: one bit operation on the receiver, which never yields
  (`lower/stmt.rs`, `lower/cond.rs`).
- `unbounded_loop_without_yield` is the lint's only reader of the list. A `loop` whose body
  only sets or tests flags is now reported, as `loop { x.set(A); }` already was.
- The lint has no `temp_across_yield` or `let_across_yield` rule, so there is nothing else to
  update.
- The rule's help named `idle();`, which does not build (`idle` takes the ticks to wait). It
  now names `idle(n);` / `sleep(n);`.

## Decision 2: the step 2a words, audited

#482 made `wall_placement` (`WallPlacement`) and `world.expansions` (`Expansions`) typed
bitflags, and wired both into `prefer_method_syntax` and `untyped_engine_field`. The audit
found everything in place but three things:
- **The flag-method finding's column.** Since #482 it pointed at the receiver's dot
  (`self▲.wall_placement`, `world▲.expansions`), not at its start, as it did for every
  receiver before. It now points at the receiver's first character again (`world` for
  `world.expansions`). The fix range was already right; only the reported position moves.
- **A flag method after a wide string on its line.** The lift places a fix by its column read
  as a byte offset; after `"ééé"` on the same line the column counts characters, so the fix
  would land early. The method finding is now skipped there, as `untyped_engine_field`
  already was for a field.
- **The text a cell's name replaces.** `untyped_engine_field` replaced the text from the
  finding to the first `]`, unchecked. `temp[0] = (global)[20];` builds, but the finding read
  `` `global)[20]` ``, and its rewrite does not parse; a `]` in a comment between the brackets
  cut the range short. The finding now stands only when that text is the whole cell on one
  line: `global[N]` or `root.cell[N]` (blanks allowed), with only the number between the
  brackets (`global[(20)]` too). A word glued after the `]` (`global[20]flag_set ..`) gets
  no finding either: the name would run into it.
- **Confirmed by tests:**
  - `self.data[13]`, `stack_obj.data[13]` and `global[20]` are named `self.wall_placement`,
    `stack_obj.wall_placement` and `world.expansions`;
  - a flag operator on the numbered cell is named first, then takes the method;
  - the method on `self`, `stack_obj`, `sim`, `me` (in a Sim's body) and `world.expansions`
    builds the same bytes;
  - an object's own `Expansions` type, a file `WallPlacement` type, a template's body, and
    the text `world . expansions` get no method finding.
- **`return error;`:** no stale branch, comment or hint is left in `lint.rs`.
  `prefer_question_mark` reads only `panic!()`, `unreachable_code` names it, and a bare
  `return error;` is an ordinary `return` to every rule.
- **Not suggested:** a flag method on a `for o in lot.objects()` handle. It skips the direct
  cursor lowering, so its bytes differ (#482 pins this). The list forms are not suggested
  either: no operator spells them, and folding `x.set(A); x.set(B);` into one call is a
  different rewrite (see the roadmap follow-ups).

## Consequences

- `tests/integration/v2_2_step2_lint.rs` has 9 tests. Every rewrite they apply builds
  identical bytes, and every loop body they lint builds.
- The open pack still lints clean with `--deny-warnings` and passes `fmt --check`.
- **Lift corpus** (`cache/track9/lift-panic-discard-beh`, 1,191 files lifted before #482):
  0 errors. Every finding the #479 lint reports is still reported, plus 2,563
  `prefer_method_syntax` findings on `wall_placement` and `world.expansions` operators,
  which #482 made method-ready (#481 counted 2,795 typed flag operators left, some on
  `attr[N]`).
