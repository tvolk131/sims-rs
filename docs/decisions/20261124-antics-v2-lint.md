# `.antics` v2 linter (`sims-antics lint`) and more `lift --v2` idioms

*Decided 2026-11-24.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`, `sims-antics lint`, `crates/sims-antics/src/v2/decompile.rs`
- **Track:** Track 9

## Context

v2 has a compiler, a formatter and a decompiler, but nothing that flags source that compiles yet is probably wrong or needlessly low-level. Examples:

- a `_` arm that hides an enum variant;
- a fallible call whose result is dropped;
- a `loop` that never yields;
- `self.` after `stack_obj` was reassigned.

The same pilots that drove PR D and PR E showed which shapes recur in hand-written and decompiled source.

Constraints:

- The v2 lexer, parser, AST, loader, lowering and optimizer belong to Track 8. The verifier and `compile.rs` belong to Track 7. None of them change.
- Fixtures are synthetic.

## Decision 1: lints walk the parsed AST, directives come from comments

`lint_v2_sources(files, options)` lints a set of files together, and `lint_v2_source(source, filename)` lints one. Linting works like this:

1. Each file is parsed with the real v2 parser, so a file that does not parse is an error, not a lint.
2. A symbol table is built over the whole set: enums, `fn`/`tree`/`extern` return ABIs, consts, states, BCON entries. Cross-file uses therefore count, and a template's states and a `pub const` are only judged against the files given.
3. Each rule walks the AST. Spans are the statement or expression spans the parser records: 1-based line, byte column.
4. Directives are read from line comments by a small scanner that skips strings and `raw { }` bodies. The parser is unchanged, because comments are not in the AST.
   - `// antics-lint: allow(rule, ...)` on its own line covers the next line of code.
   - At the end of a line, it covers that line.
   - `//! antics-lint: allow(rule)` covers the whole file.
   - An unknown rule name in a directive is itself reported (`unknown_lint_rule`).

Each diagnostic carries:

- `rule`;
- `severity`;
- `file`, `line`, `column`;
- `message`;
- `help`, and a `suggestion` (replacement text) where one is mechanical;
- an optional `note`.

### Rules

All rules are warnings except `enum_wildcard_all_covered`, which is an error that cannot be suppressed.

| Rule | Fires on |
| --- | --- |
| `enum_wildcard_single_variant` | A `match` (statement or expression) whose arms are all `Enum::Variant` paths of one known enum, with a `_` arm standing for exactly one variant. Suggests naming it. |
| `enum_wildcard_all_covered` | Same shape, with every variant already named. **Not suppressible.** An `allow` for it is ignored, and the diagnostic gets a note: to handle out-of-range raw values, type the value `i16` instead of the enum. |
| `dropped_bool_call` | An expression statement that is just a call whose result is a `bool` or `Result`, so the result is thrown away. The call may be a user `fn ... -> bool`, a `tree ... -> bool / Result`, an `extern` fn `-> bool / Result`, or a pure builtin (`can_afford`, `x.is_type(..)`); `debit_funds(n);` after a `can_afford` check is the usual spelling, so it is not flagged. A user definition shadows a builtin of the same name. A name with conflicting definitions is skipped, and so is a call passing a `&mut` argument (made for its out-parameters). |
| `unbounded_loop_without_yield` | A `loop` / `while true` whose body has no exit (`break` of this loop, `return`, `?`) and nothing that may yield (`sleep`, `idle`, `animate`, any call that is not a known non-yielding builtin, a `raw` block). |
| `stack_obj_clobber_after_self` | In a Sim-context body (a `tree` with `(sim: Sim)`, an interaction, a functional, or a `fn` taking a `Sim`): `self` used after `stack_obj = x` (or `find_best_interaction()`), in the same block or a nested one, before `stack_obj = self`. There, `self` names the new stack object (PR D). |
| `unused_decl` | Any of: a `let` (or `let Some`) binding never read; an object `state` never named as a field anywhere in the file set; a BCON entry never read while another entry of the same table is; a non-`pub` `const` never named. Also a `let x: T;` read before any assignment on any path. Template members and `pub const`s are not judged, since other files may use them. Names starting with `_` are exempt. A `raw { }` body that mentions the name counts as a use. |
| `frame_headroom` | When the file compiles on its own, or inside its pack: a compiled tree with more than 200 nodes (the VM addresses 251, `lower::MAX_BHAV_NODES`) or more than 12 locals. Reported at the source member with that tree's label. |
| `while_true` | `while true { }`. Suggests `loop { }`, which lowers identically. |
| `redundant_bool_return` | `if c { return true; } return false;` (or the `else` form, or inverted). Suggests `return c;` / `return !c;`. |
| `collapsible_if_chain` | An `if x == A { } else if x == B { } else if x == C ...` chain, three or more tests of the same side-effect-free place against literals, consts or enum paths. Suggests `match x { ... }`. |
| `manual_raw_has_builtin` | A `raw { }` block whose every statement is a primitive v2 spells natively: `action_string`, `refresh`, `play_sound`, `stop_all_sounds`, `go_to_relative`, `push_interaction`, `run_tree_by_name`, `manage_inventory`, `show_string`, `set_headline`, `special_effect`, `breakpoint`, `test_object_type`, `set_to_next`, `notify_idle`, `stack_obj :=`. Lines with `pad` or explicit `->` targets are byte-exact spellings v2 cannot express, so they are skipped. |
| `magic_guid_literal` | A hex literal above `0xFFFF` inside a body. Suggests a named `const`. |
| `negated_comparison` | `!(a op b)` for a comparison. Suggests the flipped operator. A note explains that v2 emits the negated compare node (PR E, L2), so the form is kept when mirroring an original's last-result polarity; allow the lint then. |

`sims-antics lint [--deny-warnings] [--json] <path...>` lints `*.antics` files, walking directories. Files inside a pack (a `pack.toml` above them) are compiled with that pack for `frame_headroom`.

- **Output.** Diagnostics print as `file:line:col: warning[rule]: message`, followed by the source line, a caret, and `help:` / `note:` lines. `--json` prints one JSON array instead.
- **Exit status.** 1 if any error-severity diagnostic is emitted, or, under `--deny-warnings`, any diagnostic at all. 0 otherwise.

**Why the AST, not the lowered `.air`.** The rules are about how the source is written. A rule that needed lowering facts (`frame_headroom`) reads the compiled trees and maps them back by label.

**Why conservative.** A lint that cries wolf gets suppressed wholesale. Every rule skips what it cannot resolve: unknown enums, ambiguous callees, unbounded-loop bodies that call anything unknown. The `unused_decl` uninitialized-read check flags only reads with no assignment before them on any path.

## Decision 2: more `lift --v2` idioms

The decompiler's translator learns more of the spellings PR E added. Each is still proven by the validation loop (compile, then check every tree), so a wrong idiom drops that tree to a lower level instead of shipping.

The idioms are listed in the PR and in `decompile.rs`.

## Consequences

- Lint findings are advisory, except `enum_wildcard_all_covered`, which marks an arm that can only run on an out-of-range value.
- New rules can be added one at a time with their own positive and negative synthetic tests.
- **What would reopen this:**
  - comments in the AST, so directives move to the parser;
  - a typed IR from the lowering, so rules could use real types instead of shapes.
