# `lift --v2` Rust polish: names, `else if`, `Motive::`, paths, flipped tests

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/decompile.rs`; lift test expectations; new
  `v2_lift_rust_polish.rs`
- **Track:** Track 9 (`lift --v2`), `.antics` v2.1 Rust-Idiom Polish, Step 1
- **Builds on:** [`20261125-antics-v2-lift-wave3.md`](20261125-antics-v2-lift-wave3.md)

## Context

After #473, lifted code compiled and proved, but still read like generated code:
- every function carried its tree ID (`have_a_meal_4098`), although `#[bhav(id = ..)]`
  already names the tree;
- an `else` holding a lone `if` was written `else { if .. }`;
- motives were numbers (`sim.motive[7]`), and constants used a field's dot
  (`Tuning.e1`);
- each typed import had its own `use` line;
- a negated test kept its parentheses even around a call (`!(inc_lt(..))`), and a
  negated comparison was never written the other way round (`!(a > b)`).

Every form below already compiled on main. The question was where each one is safe.

## Decision 1: forms that build the same bytes

These are written wherever they apply. After the round loop, the type and lint passes, a
final pass (`Model::polish`) applies the text rewrites. It keeps each one only when the
object still builds to the **identical** `.iff`, so every check carries over unchanged.

- **Function names.** A tree is called by its label in `snake_case`, without `_{id}`, when
  that short name:
  - is no other tree's in the file;
  - is no identifier of the file's code (a local, state, sound, constant, declaration, or a
    word of a `raw { }` body; strings and comments do not count);
  - is not reserved: any identifier-like string the v2 lexer, parser or lowering matches on
    (keywords, builtin calls and their options, field names), `.air`'s reserved words, and
    every callable and module of `std`.

  Generic labels keep `tree_{id}`. Calls, interaction comments and callback fields use
  the same name. A label attribute the short name spells is dropped, since the compiler
  gives that label anyway. The names are tried together, then in halves (at most 8 builds).
- **`else if`.** An `else { .. }` whose block is one `if` chain and nothing else (no other
  statement, no comment) is written `else if ..`. Only an `if`'s `else` is joined, not a
  `for` loop's. `raw { }` bodies are single tokens, so `.air` is never touched. The parser
  reads both forms as the same statement.
- **Braced imports.** `use` lines that import single names from one module become one
  sorted line: `use std::flags::{Expansions, ObjectFlags};`. A glob stays on its own line.
- **`Motive::` indices.** A motive cell is written `sim.motive[Motive::Hunger]` (also on
  `me` and `stack_obj`) for the ten motives the lowering names (`motive_by_name`). Other
  indices stay numbers. A unit test pins the table to the lowering's.
- **`Table::entry`.** The object's own constants are written `Tuning::e1` and library ones
  `library_consts::g260_0`. Raw `.air` keeps the dot, which is its own syntax. One gap:
  lint `unused_decl` counts a read written `Table.entry` (in v2 code or `raw { }`) but not
  `Table::entry`. So when raw code reads a table by name, an entry that only v2 code reads
  would be reported as never read. In that case (a file whose table is read both ways
  and whose lint reports such an entry) the lift writes that entry `Table.entry` again. It
  has the same bytes. Once the lint counts paths, this fallback can go.
- **`!call()`.** A negated call loses its parentheses: `!inc_lt(&mut temp[0], 6)`. A
  comparison keeps them, and so does a double negation (`!(!x.is_empty())`).

Motives, constant paths and `!call()` are written by the generator itself. Each lowers to
the same `.air`, checked with `expand`, so the round loop sees the same bytes as before.

## Decision 2: a flipped comparison only where the tree stays proven

`!(a > b)` lowers to the `>` node with its branches swapped. `a <= b` lowers to a `<=` node
with the branches as written. Both take the same branch, but the node's own result is
opposite, and a later node can observe that (a call, a primitive, `sleep`, the tree's end).
So the flip is never a byte rewrite:

- **Strict mode** keeps every `!(a op b)`, because the flipped node is never the same bytes.
- **Behavioral mode** flips a test that directly follows `if` or `while`, with one
  comparison inside and nothing else at the top level. It does so only in a tree already
  proven *equivalent*: an identical tree would stop being identical. The file is rebuilt
  and every tree checked again. A tree whose check changes in any way gets its negations
  back, and the rest are tried again (3 rounds at most). Nothing is kept unless every
  tree's check is unchanged.
- **Kept tests.** A test the lint still reports as `negated_comparison` is one the lift keeps
  on purpose: it is the original's compare node. It gets the `allow` the rule's own note
  asks for, written as
  `// antics-lint: allow(negated_comparison): the original's compare node, kept as it is`.

Most negated comparisons sit in trees the checker cannot prove (behavioral: about 20,700
of 26,206 trees are inconclusive). Flipping those would trade a node the checker matched
for one it cannot, so they stay negated.

## Results

The corpus of 1,191 objects and 26,206 trees, lifted in both modes. The base is `e6099e5bb`.

| Item | Behavioral before → after | Strict before → after |
|---|---|---|
| Functions named with `_{id}` (of 25,617) | 25,617 → 1,793 | 25,617 → 1,778 |
| `else { if .. }` that could be `else if` | 630 → 0 | 7 → 0 |
| `else if` | 13,293 → 13,923 | 349 → 356 |
| `.motive[N]` with a named motive | 4,420 → 0 (4,420 `Motive::`) | 536 → 0 (536 `Motive::`) |
| `!(a op b)` | 10,854 → 10,827 (27 flipped) | 1,418 → 1,418 |
| `!(call(..))` | 1,399 → 0 | 75 → 0 |
| Own constants `Table.eN` in v2 code | 31,063 → 3 (31,060 `Table::eN`) | 2,299 → 0 (2,299 `Table::eN`) |
| Library constants `library_consts.x` | 2,919 → 0 (2,919 `::`) | 338 → 0 (338 `::`) |
| `use` lines | 3,565 → 2,599 (602 braced) | 1,594 → 1,556 (31 braced) |

- **Verdicts are unchanged.** Behavioral: 870 identical, 4,651 equivalent, 20,684
  inconclusive. Strict: 884, 4,637 and 20,684. There are 0 DIFFERENT, 0 errors or
  timeouts, and 0 fmt-dirty files.
- **No tree got worse.** No tree lost its proof or its idiomatic form, and no file gained
  raw code (14,553 and 3,058 inline raw statements, as before).
- **Lint totals went down.** Behavioral 266 → 263 and strict 13 → 6 (`negated_comparison`
  3 → 0 and 7 → 0). `untyped_engine_field`, `untyped_engine_constant`,
  `prefer_method_syntax` and `prefer_question_mark` stay 0.
- **Runtime is the same.** Each pair below ran back to back on a quiet machine, with 10
  workers.

  | Mode | Wall time, base → this change | CPU time |
  |---|---|---|
  | Behavioral | 70.9 s → 70.8 s | +8% |
  | Strict | 91.1 s → 90.0 s | +2.5% |

  The added CPU time is the rename and polish builds and the behavioral re-check.
- **Why some functions keep `_{id}`.** In behavioral mode 1,793 do. Most are keywords or
  reserved words: `main` (705), `init` (159), `load` (114), `break`, `sleep`. Others
  clash with a library tree the file declares under the same name (`clean`, `load_tree`,
  `semi_global`). 154 are generic `tree_{id}`. The rest share a label or clash with a
  state, sound or local.
- **Why the coordinator's estimates were higher.** The estimate of 1,236 `else if` sites
  counted every `else {` followed by an `if`. 2,973 of those blocks hold more statements
  after the `if`, so they cannot be joined. The 9,343 negated tests sit mostly in
  inconclusive trees, or in strict output, where flipping would change the bytes
  (Decision 2).

## Consequences

- Lift test expectations that pinned `name_{id}`, `Table.eN`, numbered motives or
  `!(call(..))` now expect the new spellings.
- **Step 2** (after Track 8's `panic!()` and `let _ = expr;`): `return error;` becomes
  `panic!();`, and `if call() {}` becomes `let _ = call();`.
- **Not done:** a negated comparison in an unproven tree stays negated (see Decision 2).
  Making it safe needs the checker to prove more trees, or a rule that the node's result
  is never observed, as lint `negated_comparison` proves for its narrow case.
