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
4. Directives are read from line comments by a small scanner that skips strings and block comments, and skips `raw { }` bodies whole, using the lexer's `raw` tokens (they follow `.air` comment rules). The parser is unchanged, because comments are not in the AST.
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
| `enum_wildcard_single_variant` | A `match` (statement or expression) whose arms are all `Enum::Variant` paths of one known enum, with a `_` arm standing for exactly one variant. Suggests naming it. The enum is resolved in the match's file: its own declarations, then its `use` imports under their alias, as the module loader resolves them (so only when the file is in a pack on disk). A name that resolves to no single declaration is skipped. |
| `enum_wildcard_all_covered` | Same shape, with every variant already named. **Not suppressible.** An `allow` for it is ignored, and the diagnostic gets a note: to handle out-of-range raw values, type the value `i16` instead of the enum. |
| `dropped_bool_call` | An expression statement that is just a call whose result is a `bool` or `Result`, so the result is thrown away. The call may be a user `fn ... -> bool`, a `tree` or `extern` fn `-> Result` (a `-> bool` tree is often run for what it does, so it is not flagged), or a pure builtin (`can_afford`, `x.is_type(..)`); `debit_funds(n);` after a `can_afford` check is the usual spelling, so it is not flagged. A user definition shadows a builtin of the same name. A name with conflicting definitions is skipped, and so is a call passing a `&mut` argument (made for its out-parameters). |
| `unbounded_loop_without_yield` | A `loop` / `while true` whose body has no exit (`break` of this loop, `return`, `?`) and nothing that may yield (`sleep`, `idle`, `animate`, any call that is not a known non-yielding builtin, a `raw` block). |
| `stack_obj_clobber_after_self` | In a Sim-context body (a `tree` with `(sim: Sim)`, an interaction, a functional, or a `fn` taking a `Sim`): `self` used after `stack_obj = x` (or `find_best_interaction()`), in the same block or a nested one, before `stack_obj = self`. There, `self` names the new stack object (PR D). |
| `unused_decl` | Any of: a `let` (or `let Some`) binding never read; an object `state` never named as a field anywhere in the file set; a BCON entry never read while another entry of the same table is (skipped when a `raw` block reads `bcon[N]` / `const[N]` or an `extern` tree is declared, since either may read entries by number); a non-`pub` `const` never named. Also a `let x: T;` read before any assignment on any path. Template members and `pub const`s are not judged, since other files may use them. A `let` is not judged when a later `raw` block addresses `local[N]`, which may be its slot. Names starting with `_` are exempt. A `raw { }` body that mentions the name counts as a use. |
| `frame_headroom` | When the file compiles on its own, or inside its pack: a compiled tree with more than 200 nodes (the VM addresses 251, `lower::MAX_BHAV_NODES`) or more than 12 locals. Reported at the source member in the same object with that tree's pinned id or (unique) label, else at the object. |
| `while_true` | `while true { }`. Suggests `loop { }`, which lowers identically. |
| `redundant_bool_return` | `if c { return true; } return false;` (or the `else` form, or inverted). Suggests `return c;` / `return !c;`. |
| `collapsible_if_chain` | An `if x == A { } else if x == B { } else if x == C ...` chain, three or more tests of the same place, where the `match` would compile: integer literals against `temp[N]`, `param[N]` or an `i16` state, or variants of a resolved enum against a state of that enum. Places of other or unknown types (`stack_obj`, locals) are not judged. Suggests `match x { ... }`. |
| `manual_raw_has_builtin` | A `raw { }` block whose every statement is a primitive v2 spells natively with the same operands: `action_string`, `refresh`, `go_to_relative`, `run_tree_by_name`, `manage_inventory`, `show_string`, `set_headline`, `special_effect` (all only with `key: <literal>` operands in the ranges the v2 builtin accepts, and `special_effect` without flag `0x20`, which reads a local), and argless `breakpoint()` / `notify_idle()`. Lines with `pad` or explicit `->` targets are byte-exact spellings v2 cannot express, so they are skipped. Primitives whose v2 spelling takes other operands are not listed: `play_sound` (v2 names a declared `sound`), `stop_all_sounds` and `push_interaction` (v2 picks the target from the tree's context), `test_object_type` as a statement, `set_to_next`, and `stack_obj :=`. |
| `magic_guid_literal` | A hex literal above `0xFFFF` inside a body. Suggests a named `const`. |
| `negated_comparison` | `!(a op b)` for a comparison. Suggests the flipped operator. A note explains that v2 emits the negated compare node (PR E, L2), so the form is kept when mirroring an original's last-result polarity; allow the lint then. |

`sims-antics lint [--deny-warnings] [--json] <path...>` lints `*.antics` files, walking directories. `.air` files share the extension and are skipped; a file starting with `const` (both languages have it) is linted when it parses as v2 or as neither, so its syntax error is reported. Files inside a pack (a `pack.toml` above them) are compiled with that pack for `frame_headroom`.

- **Output.** Diagnostics print as `file:line:col: warning[rule]: message`, followed by the source line, a caret, and `help:` / `note:` lines. `--json` prints one JSON array instead.
- **Exit status.** 1 if any error-severity diagnostic is emitted, or, under `--deny-warnings`, any diagnostic at all. 0 otherwise.

**Why the AST, not the lowered `.air`.** The rules are about how the source is written. A rule that needed lowering facts (`frame_headroom`) reads the compiled trees and maps them back by label.

**Why conservative.** A lint that cries wolf gets suppressed wholesale. Every rule skips what it cannot resolve: unknown enums, ambiguous callees, unbounded-loop bodies that call anything unknown. The `unused_decl` uninitialized-read check flags only reads with no assignment before them on any path.

## Decision 2: more `lift --v2` idioms

The decompiler's translator learns more of the spellings PR E added:

- `match` for a chain of three or more literal tests of one place, and `else if`;
- `while x inc_lt n` / `dec_gt` as `inc_lt(&mut x, n)` / `dec_gt(..)`;
- `set_dynamic_sprite(index:, value:)` in a Sim context;
- `self.slot[i]`, `param[n]` as an assignment target, and `p.person[i]` / `p.kind` inside `for p in lot.sims()`;
- `push_interaction(.., object: x)`;
- `stack_obj.run_functional(n)`;
- `notify_out_of_idle()`.

Several of these lower to a different instruction sequence than the original (an extra `local[0]` write, a `stack_obj` restore). Each is still proven by the validation loop (compile, then check every tree).

The new idioms are gated per tree. A tree that used one and is not proven is first retried without them, at the same level, before it drops a level. So an idiom that fails can make that tree no worse than before.

`GUID_*` constants the decompiler declares but no body names are blanked out, keeping line numbers, so the output does not trip `unused_decl`.

## Consequences

- Lint findings are advisory, except `enum_wildcard_all_covered`, which marks an arm that can only run on an out-of-range value.
- New rules can be added one at a time with their own positive and negative synthetic tests.
- **What would reopen this:**
  - comments in the AST, so directives move to the parser;
  - a typed IR from the lowering, so rules could use real types instead of shapes.
