# `.antics` v2: five new lint rules, exact `unused_decl` for constants, one v2 detector

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`, `crates/sims-antics/src/v2/mod.rs` (`is_v2_source`), `sims-antics lint`
- **Track:** Track 7
- **Supersedes:** row 55 of [20261124-antics-v2-pilot-fixes.md](20261124-antics-v2-pilot-fixes.md) (a `const` whose value a `raw` block writes as a literal counts as used)

## Context

The Round 4 and Tier B pilots turned up mistakes that compile, or that fail only at `check`, with nothing pointing at them first:

- **A shadowed constant.** A block `const` reused the name of a file-level `const`. The tree then wrote a different attribute from the one the author meant.
- **Missed unused constants.** `unused_decl` missed `const HOUR = 0;` because a `raw` block indexed `[0]`. The value-literal heuristic (pilot-fixes row 55) dates from before lowering resolved v2 constant names in `raw` blocks. Lowering has resolved them since PR H.
- **Copied `std::globals` declarations.** Hand-copied `extern mod` blocks repeat trees that `std::globals` already declares.
- **Untyped cells.** Decompiled and hand-written source reads cells by number (`self.data[11]`, `sim.person[58]`, `global[0]`) where v2 has a typed name.
- **Late errors.** A `raw { }` body that is not valid `.air`, and `me.<field>` in an object's own tree, were only reported by `check` / `expand`.

Separately, `check`, `equiv` and `fmt` used `is_v2_source`, which looked at the first word of a file. So a v2 file that starts with `const` (two of the Round 4 pilots) was read as `.air`. `lint` worked around this with its own `is_lintable_v2`.

## Decision 1: one detector

`is_v2_source` reads the file's first word twice, after skipping whitespace and `//` and nested `/* */` comments:

1. **As `.air`.** It also skips `#` lines, which `.air` reads as comments, including `#[..]` lines. If the first word is `tree`, `interaction`, `bcon` or `str`, the file is `.air`.
2. **As v2.** It skips `#[...]` attributes instead (balanced brackets, strings skipped). If any word follows, the file is v2: `use`, `const`, `fn`, `object`, `extern`, `enum` and so on. A mistyped keyword then gets the v2 parser's diagnostic.

A file with no word at all is not v2.

`check`, `equiv`, `fmt` and `lint` all call it. `is_lintable_v2` is removed. `verify` and `expand` always compile as v2.

## Decision 2: the rules

| Rule | Severity | Fires on | Does not fire on |
| --- | --- | --- | --- |
| `shadowed_const` | warning | A block `const NAME = ..;` / `const (A, B) = ..;`, or a `const n: T` parameter, whose name is a file constant or a constant of the enclosing object or template. Also an object constant named like a file constant. | Names imported with `use` (not resolved here). |
| `unused_decl` (constants) | warning | File, object and block constants no code names. | `pub` constants; template constants; `_` names; a name written anywhere in the file set other than as a `const` name (bodies, `guid:`, indices, other constants); a name a `raw` block uses; a block binding whose selector returns, breaks or continues (it is control flow). |
| `duplicate_std_global` | warning | An `extern mod` function (top level, object or template) whose ID, parameter types and slots, and return ABI equal a `std::globals` function. Names are ignored; the help lists the `std::globals` parameter names when they differ, since named arguments must change. | The same ID with another ABI. |
| `untyped_engine_field` | warning | A cell read or written by an index written as a number, where the cell has a typed name lowering maps to the same operand (tables below). | Indices with no name; a constant index (a specialized `const` parameter may stand for several cells); anything in `raw { }`; `world.x` (it parses to `global[N]`, so the source text is checked); a handle name a parameter or local rebinds (`let me`, `self: Sim`). |
| `raw_syntax` | error, cannot be allowed | A `raw { }` body that does not parse as `.air`. | See Decision 4. |
| `object_tree_me_field` | error, cannot be allowed | `me.<field>` in an object's own tree (`init`, `main`, `load`, `reset`, `cleanup`, or a tree with no Sim parameter and no `context = inherit`). There `me` is the object, and lowering rejects every field. | `me.attr[N]` / `me.data[N]` and other indexed cells; Sim contexts; `context = inherit` trees; a local or parameter named `me`; a file set with a BCON table (own, template or `extern bcon`) named `me`. |

Block `const` bindings are specialized away by the parser, so both constant rules read them from the token stream. A `const` followed by `NAME =`, `( .. ) =` or `NAME :` is a binding unless it is a declaration the AST holds. Its scope is the last object, template or helper that starts before it.

`raw_syntax` and `object_tree_me_field` cannot be allowed, because an `allow` cannot make the file build. The note says so.

## Decision 3: `untyped_engine_field` fires only where the typed name is a drop-in

Each entry was checked against lowering (`lower/expr.rs`, `lower/lvalue.rs`, `lower/tree.rs`) and the `.air` operand parser. An integration test compiles each flagged statement and its suggestion, and asserts byte-identical IFFs.

- **`global[N]`:** every `world.<name>` (`hour` 0, `day` 1, `selected_sim` 3, `time_of_day` 4, `minute` 5, `second` 6, `month` 7, `year` 8, `house` 10, `paused` 17, `days_played` 28, `free_will` 30). Reads and writes.
- **`self.data[N]`:**
  - read and write: `graphic` 0, `room_impact` 7, `wall_placement` 13, `broken` 15, `in_use` 25, `prep_score` 31, `cook_score` 32;
  - read only: `room` 29, and `id` 11 (see below).
- **A Sim handle's cells:** the Sim parameter of the tree, interaction, functional or guard, and `me` where `me` is the Sim.
  - `person[N]`, read and write: `posture` 0, `priority` 33, `pets_outside` 34, `job_type` 56, `job_level` 57, `age` 58, `family` 61. `visitor_kind` 32 is read only.
  - `data[N]`, read only: `room` 29, and `id` 11 and `container` 2 (see below).
  - `slot[0]`, read only: `hand`.
- **Not flagged:**
  - `flags` (`data[8]`, not the `data[40]` in the brief), `allowed_heights` (`data[4]`) and `placement` (`data[42]`). Their names are typed bitflags, so an `i16` use of the cell does not type-check with the name.
  - `id` and `container`, except where used for (in)equality or assigned as a whole. They are `ObjectId`s, which take no arithmetic.
  - A store through a read-only name.

Across corpus-v2, the rule reports 996 findings in 426 decompiled files. With every suggestion applied, all 426 files still pass `check`. The findings are real; the decompiler could print the typed names.

## Decision 4: `raw_syntax` parses like lowering, and does not resolve names

The body is wrapped the way lowering validates it (`tree 4096 "__raw_check" { .. }`). It is preceded by the `const` lines lowering would declare for it:

- each file or object constant, and each BCON entry `TABLE.entry` of an own table or an `extern bcon`, that the code names;
- less the constants the block declares itself (by full name, `K` or `T.e`);
- less the `.air` locals (`let` / `local`) any raw block of the tree declares, since `.air` locals are tree-wide and win over constants.

A constant brought in by `use` from another module is not resolved here. So when the parse stops at a name, or at the `.` of a `T.e`, that name is declared with a placeholder value and the body is parsed again (at most 32 names). Only an error that remains is reported. A stray word (`temp[0] := 1 nonsense;`) still fails at the same place, so it is still reported.

## Consequences

- On open-pack and the `test_runner` fixtures, `lint --deny-warnings` still reports nothing.
- Round 4 (21 files) reports 13 findings:
  - `untyped_engine_field` ×10;
  - `unused_decl` ×2 (one unused constant in each of two files);
  - `shadowed_const` ×1 (a block `const` in one file reuses a file constant's name).
- Linted file by file, corpus-v2 adds nothing beyond `untyped_engine_field`.
- A constant used only as a literal in raw code (`attr[7]` for `SLOT = 7`) is now reported as unused. Write the name.
