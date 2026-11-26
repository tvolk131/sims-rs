# `.antics` v2 lint: `panic!()`, `let _ = expr;` and `Table::entry` reads

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`; new `tests/integration/v2_lint_wave6.rs`.
  No change to `v2/decompile.rs`, `v2/lower/*` or `v2/parser.rs`.
- **Track:** Track 7 (lint), `.antics` v2.1 Rust-idiom polish, Step 2
- **Builds on:** [`20261125-antics-v2-panic-and-discard-let.md`](20261125-antics-v2-panic-and-discard-let.md)
  (#475), [`20261125-antics-v2-lift-rust-polish.md`](20261125-antics-v2-lift-rust-polish.md)
  (#476), [`20261125-antics-v2-lint-collapsible-else-if.md`](20261125-antics-v2-lint-collapsible-else-if.md)
  (#474)

## Context

#475 added two spellings, and #476 started writing a third. The lint did not know any of them:

- `panic!()` parses to the `return error;` statement (`V2Stmt::ReturnError`, `panic: true`).
  Every rule already treated it as an exit, but two rules named it wrongly or missed it:
  - `unreachable_code` said "`return error` on line N" after a `panic!()`;
  - `prefer_question_mark` matched only a written `return error;`.
- `let _ = expr;` parses to a `let` named `_`. It discards a result on purpose, so no rule
  should flag the `_`. But the rules that read inside a call statement should still read
  inside the discarded expression.
- #476 writes BCON reads as `Tuning::e1` (a two-segment path), not `Tuning.e1`.
  `unused_decl` counted only the dot form. So the lift kept a dot fallback at 3 corpus sites
  where an entry was read only by path.

## Decision 1: `panic!()` in the rules that read exits

- **`unreachable_code`** names what ended the block: "`panic!()` on line N always leaves
  this block". A written `return error;` keeps "`return error`".
  - The old "`?`" wording is gone. `from_try` is set only by lowering's own desugaring, never
    by the parser, so no linted statement has it.
- **`prefer_question_mark`** matches `if !f(..) { panic!(); }` and
  `if f(..) {} else { panic!(); }` as well as the `return error;` forms. It still suggests
  `f(..)?;`. As before, the finding stands only when the edited file builds the same bytes.
  - A written `return error;` is skipped when a binding named `error` is in scope, because
    then it returns that binding.
  - `panic!()` is always the error exit, as `?` is. So the binding does not block it.
  - The rule summary now leads with `panic!()`.
- **The rest:**
  - `unbounded_loop_without_yield`'s exit test, `stmt_leaves` and `stmt_span` take
    `ReturnError` as a whole, so `panic!()` already behaved the same;
  - no other rule's text mentions `return error`.

## Decision 2: `let _ = expr;`

- **`unused_decl`** never flags `_`. The unused-local check already skipped names starting
  with `_`. Tests now pin this for calls, `Result` and `bool` calls, and literals.
- **Reads inside `expr`** count for `unused_decl` and `uninitialized_local_read`. Both walk
  every `let` initializer, so this held already; the tests pin it.
- **`let _ = relationship(.., value: n);`** builds the same as the call statement. Like the
  statement, it writes `n` only when the relationship row exists, so it is no sure write
  for `uninitialized_local_read`. Before, the initializer counted as one.
- **`prefer_method_syntax`** reads `let _ = f(..);`, so `let _ = play_sound(5, stack_obj);`
  suggests `stack_obj.play_sound(5)`. It reads only a call that is the whole initializer,
  just as it reads only a whole call statement.
- **`untyped_engine_field` and `untyped_engine_constant`** walk every expression, so
  `let _ = random(self.data[41]);` and `let _ = self.flags2 flag_test 6;` report.
- **`dropped_bool_call`** stays off for `let _ =`, which is the explicit discard. Its help
  now offers it: "... or drop it on purpose with `let _ = f(..);`". For `x.is_type(..)` the
  help now spells the receiver (`let _ = x.is_type(..);`); bare `is_type` is no callable.

## Decision 3: `Table::entry` counts as a BCON read

Each file's tokens give its two-segment paths `A::b`. A BCON table is in scope only in its
object's body and in the body of the template the object is built `from`. So each path is
recorded with the body it stands in, as `(body, A, b)`. `body` is one of:
- `object:<file>:Name`, so same-named objects in two files stay apart;
- `template:Name`;
- `module`, for a path outside any body. A module-level helper is inlined into whatever
  object calls it, so such a path counts for every object.

These are skipped:
- a segment of a longer path (`a::A::b`);
- a called path, `globals::init_object()` or `((globals::init_object))()`. A call is never
  a BCON read, and without this `std::globals` calls would hide the entries of a table
  labeled "Globals".

Lowering gives a path to a type before a BCON table: an enum or bitflags type of the same
name in scope wins `A::b`. So a path is also skipped when `A` is such a type where it
stands:
- one the file's module sees, from the resolver: the file's own items, imports, globs and
  aliases;
- a `std` prelude type (`Height`, `Placement`);
- one the body declares;
- for an object body, one its `from` template declares.

If the module does not resolve, every name the file imports is taken for a type. This
leans to a missed finding.

A BCON entry of object `O` counts as read if either holds:
- its name follows a `.` anywhere (as before);
- for one of the table's names `T` (`object_type_ident(label)`, `snake_case_ident(label)`),
  one of these was recorded: `(object:<O's file>:O, T, entry)`, `(module, T, entry)`, or
  `(template:F, T, entry)` where `F` is `O`'s `from` template. The template case does not count when `O` itself declares a type named
  `T`: lowering merges the two bodies, and `O`'s type wins the path in the template's trees
  too.

A file with no candidate path is not resolved again, so lint time is unchanged for it.

Known limits:
- A read in a template file is seen only when that file is linted too. The same holds for
  `.` reads.
- Templates are keyed by name alone. When two linted modules each declare a template
  `Base`, either one's reads count for an object built from `Base`. That leans to a missed
  finding.
- A template imported under an alias (`use m::Base as B; object .. from B`) is keyed by
  its own name, so its path reads are not matched. This leans to a false finding. It is
  rare.
- A template's reads count even when the object replaces the tree that holds them (its
  own `main`). That leans to a missed finding.
- Without a pack (no `pack.toml`), the resolver runs with the default config, the same
  config the lint's own lowering uses. So a `crate::` import does not resolve there.

Matching exact pairs, not entry names alone, keeps `Motive::Hunger` or `Other::high` from
hiding an unread `Hunger` or `high` entry. Raw blocks already counted both `.` and `::`
spellings through their `.air` member scan.

**`extern bcon` tables** have no unread-entry check. The lint cannot see which entries the
game's own trees read. Path reads of them are recorded like any other, so a check could be
added later. Adding one now would be a new finding class on every lifted file that imports
a library table, so it is left as a follow-up.

**Track 9 follow-up:** the lift's 3-site dot fallback (#476) no longer fires, and can go.
- With the coordinator's OK, this PR changes one expectation in `decompile.rs`: the test
  `raw_lines_name_the_objects_own_bcon_entries` now expects `temp[0] = Tuning::e0;`.
- The fallback code itself is untouched.

**A table read only by path is judged again.** The check judges a table's entries only
when at least one entry is read by name. A table no entry of which is read may be read by
number elsewhere. Before this change, a table that #476's lift reads only through
`Table::entry` looked unread, so none of its unread entries were reported. Now they are,
as they were when the lift wrote dots.

## Consequences

- `tests/integration/v2_lint_wave6.rs` has 27 tests: positive and negative cases for each
  decision. Every rewrite the tests apply builds identical bytes.
- The open pack lints clean with `--deny-warnings`, passes `fmt --check`, and builds
  byte-identical to main.
- **Lift corpus** (`cache/track9/lift-rust-polish-beh`, 1,191 files from main after #476):
  - 0 errors; every rule but `unused_decl` is unchanged against main;
  - BCON `unused_decl` findings: 17 false positives go (entries read only as
    `Table::entry` in a table also read with a dot, as in `DecorationStates::e8`);
  - 508 entries no code reads are reported again, in tables read only by path. Main's
    lint missed them; on the wave-3 lift, which wrote dots, main reported 913 such
    findings in 99 files. Now: 619 in 98 files.
