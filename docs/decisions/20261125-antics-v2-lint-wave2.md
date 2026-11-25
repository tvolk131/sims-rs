# `.antics` v2 lint, wave 2: two new rules, PR I-3 raw forms, two `unused_decl` fixes

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`, `sims-antics lint`
- **Track:** Track 7 (Wave 1, Stream 4a of the `.antics` v2.1 milestone)
- **Builds on:** [20261124-antics-v2-lint.md](20261124-antics-v2-lint.md), [20261125-antics-v2-new-lints.md](20261125-antics-v2-new-lints.md)

## Context

PR I-3 (#466) and the lift's inline-raw cleanup (#463) left three things for `lint.rs`:

- **`set_to_next(.., dest: x)`** both reads and writes `x`. The lint did not model the write.
- **BCON entries named in raw code.** The lift now writes `Table.entry` / `Table::entry` inside `raw { }`. `unused_decl` did not count those as reads, so an entry read only from raw code was reported unused. This is the follow-up the inline-raw cleanup ADR asked for.
- **New forms.** v2 now spells several primitives that used to need a raw block. `manual_raw_has_builtin` did not know them.

Two more rules were asked for:

- a warning for an `allow` directive that suppresses nothing;
- a warning for code that can never run.

## Decision 1: `set_to_next(.., dest: x)` is a read, then a write

`set_to_next` joins the builtins whose arguments write a local (`random_into`, `animate`, `relationship`). Its `dest:` argument is the only one that is both read and written.

- **`uninitialized_local_read`** sees the read at the call, then the write. A `let c: ObjectId;` stepped before any assignment is reported at `dest: c`. The VM starts from cursor 0 there, and the fix is `= 0`. A read of `c` in another argument of the same call is reported too.
- **`unused_decl`** counts `dest: x` as a read.
  - It now also tells pure writes apart: a local that only `random_into(x, ..)`, `animate(.., local: x)` or `relationship(.., value: x)` writes, and nothing reads, is unused.
  - Before, any mention counted as a read.
  - No open-pack or corpus file changes from this.
- **`param[N]` and `temp[N]`** destinations are not locals. They get no `unused_decl` or `uninitialized_local_read` finding, and the step marks them written, like an assignment.

## Decision 2: BCON entries read by name inside raw code

`Symbols` records every word after a `.` or a `::` in a raw block's code, with comments and strings blanked, as a field name. `unused_decl` already treats such a name as a read of a BCON entry.

- This is the same rule as for v2 code, where any `.entry` counts.
- `bcon` tables are what the rule judges. `extern bcon` entries are never reported unused: another file owns them.

**Corpus:** 52 false `unused_decl` findings in `corpus-v2-behavioral` are gone. Each one is an entry read only as `Table.eN` in a raw `expr(..)`.

## Decision 3: PR I-3's forms in `manual_raw_has_builtin`

The rule still fires only when every statement of a `raw { }` block has a v2 spelling. A block with a label, an explicit edge (`->`), a brace, or a `pad` byte is left alone, as before.

These statements are now spelled:

| Raw `.air` statement | v2 | Left raw |
| --- | --- | --- |
| `remove_object(T)`, `T` = `me` / `self` / `0` / `stack_obj` / `1`, with `flags: 0..255`, `no_yield:`, `skip_resets:` | `remove_object(..)` | target `2`; any other option; out-of-range flags |
| `find_best_object(kind)`, a searched function by name or number, with `flags:` (named or positional) | `find_best_object(..)` as an `if` / `while` test | `origin:`; never-searched functions (`stand`, `eat`, ...); `category4` |
| `set_to_next(mode, dest: X)` / `into: X`, mode 0..12 except `beside` (9), with `guid: <literal>`; `X` = `temp[0..7]`, `param[N]`, or a v2 `i16` / `ObjectId` local | `set_to_next(.., dest: ..)` | `temp[8]` and up; `stack_obj` or no `dest:`; `beside` / `local:`; typed handles, bools, parameters, a raw block's own locals; a GUID by name |
| `X := scope(21, 9)` / `scope(21, 11)` / `scope(33, F)` into `temp[0..7]` or an `i16` local | `stack_obj.object_type` / `.sub_index` / `job_field(F)` | other definition words; any other expression; `my.attr[..]`, `local[N]` targets |
| `X := scope(35, N)` into a `bool` local | `stack_obj.has_function[N]` | an `i16` or `temp` target (the read is a `bool`) |

Notes on the table:

- **Not covered.** A store into the stack object, and the scope cells inside tests (`if scope(21, 9) == 2 {`), are outside this rule. Its unit is a whole raw block of statements.
- **Locals.** The lint knows a local's kind from its `let` (the declared type, or an integer or bool literal initializer). Parameters and `for` bindings count as "other".

**Corpus:** `corpus-v2-behavioral` has 577 findings in 218 files: 541 are `raw { remove_object(N, flags: F); }` and 36 are `raw { temp[k] := scope(21, 11); }`.

With every suggestion applied, all 218 files still lower:

- **188 files** lower to the same `.air`, after normalizing `me`/`stack_obj` to `0`/`1` and dropping `flags: 0`.
- **The other 30 files** differ only where the optimizer, no longer blocked by the raw block, merges the identical tails of two branches (`remove_object(..); return true;`) into one after the `if`. That pass preserves behaviour.

## Decision 4: `stale_lint_allow`

**What it reports.** It is a warning, placed on the directive's comment. It fires for each rule named in an `antics-lint: allow(..)` directive when that rule has no finding on what the directive covers:

- for a line directive, its target line, its block, or its item's attributes, exactly as suppression works;
- for `//!`, the file;
- for a directive with no code after it, nothing.

**Which findings count.** They are the file's findings after the compiler gate of the unsuppressible errors, and before suppression. So an allow is not stale while the finding it silences exists.

**Directive handling:**

- **Lists.** A comma-separated list is judged rule by rule.
- **Unknown names.** These keep `unknown_lint_rule` and are not judged.
- **Plain comments.** A comment that is not a directive is ignored.

**Unsuppressible rules:**

- An allow of `enum_wildcard_all_covered`, `raw_syntax` or `object_tree_me_field` that has a finding is not stale. The finding stands with its "cannot be allowed" note.
- One with no finding is stale, and carries the same note.

**Only on files that build.** The check runs only on a file that compiles. `frame_headroom`, `dialog_local_slot_mismatch`, and `unused_decl`'s `$Local:N` reading need the build. When the build fails those rules are silent, so their allows cannot be judged.

**Allowing it.**

- An `allow(stale_lint_allow)` is never itself judged.
- A `//! antics-lint: allow(stale_lint_allow)` silences the rule for a file.
- An end-of-line directive silences it on its own line.

**open-pack:** both allows are used, so it is still 0 findings.

**Corpus:** 84 findings, all on `// antics-lint: allow(unbounded_loop_without_yield)`, which the lift writes above `loop { raw { breakpoint(..) pad 0xff; } }`. The lint never reports such a loop, because a raw block may yield. So the allow suppresses nothing, and the finding is correct. A follow-up for Track 9: the lift can stop writing it.

## Decision 5: `unreachable_code`

**What it reports.** It is a warning on the first statement of a block after one that always leaves the block. Such a statement:

- is `return`, `return error`, `break` or `continue`; or
- is an `if` with an `else` whose every branch leaves (recursively, so an `else if` chain counts); or
- is a `match` with a `_` arm whose every arm leaves.

It reports only the first unreachable statement of a block, and notes which statement leaves and on what line. Lowering already drops these statements (`opt.rs` `drop_unreachable`). The warning tells the author that the code they wrote does nothing.

**What it does not report:**

- **Fall-through cases.** An `if` without `else`; a `match` without `_` (a value no arm names falls through); `cond?;` (an `if` with no `else`); loops, `with lock`, `defer` and `animate` (each may go on).
- **Raw labels.** No statement is reported when any `raw { }` block from the unreachable statement to the end of the block, nested blocks included, defines an `.air` label (`@name:` outside comments and strings). A jump in another raw block may land there. A raw block without a label is as unreachable as any other statement.
- **A trailing return.** A lone `return` that ends a block right after an `if` / `match` whose every branch leaves is not reported. It is the trailing return written for the reader, and a direct `return; return x;` is still reported.

**Corpus:** 0 findings in `corpus-v2-behavioral` and in open-pack.

## Consequences

- `LINT_RULES` gains `unreachable_code` and `stale_lint_allow`. Both are warnings and can be allowed.
- `lint --deny-warnings` on open-pack stays clean.
- On `corpus-v2-behavioral` (1191 files, each linted alone), there are no errors or panics. Findings go from 477 to 1086:
  - `unused_decl` falls by 52 (Decision 2);
  - `manual_raw_has_builtin` gains 577 (Decision 3);
  - `stale_lint_allow` gains 84 (Decision 4).
  - Nothing else changes.

## Follow-ups

- **Track 9 (lift):**
  - Stop writing `allow(unbounded_loop_without_yield)` above a loop whose body is a raw block. The lint does not report it (84 corpus files).
  - Lift `raw { remove_object(..); }` and `raw { temp[k] := scope(21, 11); }` to the v2 spellings.
- **Not covered yet:**
  - `set_to_next(.., dest: stack_obj)` and plain `set_to_next(mode)` in raw. v2 spells both, but the earlier raw table left `set_to_next` out on purpose, and this wave keeps to the asked forms.
  - The definition cells inside raw tests.
