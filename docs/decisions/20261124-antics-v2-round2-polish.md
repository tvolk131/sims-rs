# `.antics` v2: linter polarity rules, `strings` and PR G builtins in `lift --v2`

*Decided 2026-11-24.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/{lint,decompile}.rs` and their tests
- **Track:** Track 9 (tooling round 2)
- **Follows:** `20261124-antics-v2-pilot-fixes.md`

## Context

The second round of pilot reports (Batches 1 to 4, `REPORT_R2.md`) found three problems:

- **Unsafe lint suggestions.** About 20 of the 25 `allow`s the pilots still needed were for `negated_comparison` reports whose rewrite would break the proof. A rewrite like that flips the compare node's last result before a call or a primitive that can fault, and both of those observe it.
- **`redundant_bool_return` gaps.** It stayed silent on `if c { return true; } return false;` with a comparison `c`, which PR G made safe to return directly.
- **Lift gaps.**
  - The lift wrote no `strings` members, so the 332 base-game trees that show text stayed inconclusive.
  - PR G's new builtins were still lifted as `raw {}`.

Tracks 7 and 8 own the verifier and the v2 front end and lowering. None of their files change here.

## Decision 1: lint rewrites only where the last result cannot be seen

The **last result** is the true/false outcome of the last node to run. It is observable:

- at the tree's exit;
- at a call, where the callee and the verifier see it;
- at a fault (for example, a store through a missing `stack_obj`).

| Rule | Change |
|---|---|
| `negated_comparison` | Reported only when both successors of the `!(a op b)` test are **quiet**: a plain store (`=`, `+=`, `-=`, `*=`, `&=`, `|=`, `^=`) to `temp[0..=7]`, or to a `state` field of `self` outside a Sim's body, of a value made of literals, `const` names, enum paths and such places, combined with `+ - * & | ^` only, and not the target itself. Anything else is not quiet: a call, `sleep`, a primitive, `return`, a test (which may itself end the tree), a store to a `let` (which may fold away), a local in the value (an unwritten one may fault), division, an indexed read, and anything touching `self` in a Sim's body (where `self` follows `stack_obj`). The suggestion keeps an operand's parentheses when it is itself a comparison. |
| `redundant_bool_return` | Reported for `if c { return true; } return false;` (or with `else`), comparisons and `&&` included, with the suggestion `return c;`. Only for one test or `a && b` of two (a longer chain, or `||`, lowers `return` through a local). Not reported when the condition may fold to a constant (in its `!` / `&&` / `||` structure, a `bool` literal or a comparison of two operands neither of which reads a place; a literal argument of a call does not count), in a body with `defer`, or inside `with lock`, `defer` or `animate` (where `return` leaves through the block's exit, after the test). Suggested string literals use v2's escapes only. The other polarity is never reported: `return !c;` ends on the opposite last result. |
| `uninitialized_local_read` | `relationship(value: x)` writes `x` when it returns true, unless its `flags:` are a literal with flag 4 (which writes the relationship from the local). So `relationship(..)?` and `if relationship(..) { .. }` write it, but a bare call statement does not: a missing row returns false first, and the statement goes on. Flags that are not a literal count as a write. |
| `unused_decl` (const) | A `const` named only inside `raw {}` is reported, with a note: `raw` does not resolve `const` names, and the build rejects the name there. A value written in its place still counts as a use. |
| `unused_decl` (let) | A tree that shows strings this file does not hold is treated as possibly reading every local through `$Local:N`. This covers a dialog's `STR#` 301 number past the file's own entries (the object's spliced table), and an `action_label` / `action_string` whose `table:` and `index:` (1-based) name no string in this file. `show_string` formats no tokens and reads no local. A `raw` block whose `.air` code names `dialog`, `action_string` or a bare `op` counts as showing unknown strings. |
| directives | `// antics-lint: allow(..)` and `# antics-lint: allow(..)` work inside `raw {}`, per line: trailing a line covers that line, and on its own line covers the next line of code. Inside `raw`, `//!` is never file-level, and a `#` inside an `.air` string is not a comment. A `raw` diagnostic lands on its statement's own line: the linter finds each `raw` keyword as the lexer counts columns (bytes, but characters inside a v2 string) and blanks `.air` comments in place, so a commented-out copy of a call cannot take its position, and a label before the call does not either. Comments between `raw` and its `{` are v2 comments (`//!` there is file level). |
| `unused_decl` (const in raw) | The "only named inside `raw`" advice needs the name in `.air` code: a name in an `.air` comment or string is an ordinary unused const. Likewise a literal index counts as a use of a constant's value only in `.air` code, not inside a quoted tree label. |

The verifier checked the `redundant_bool_return` change on synthetic trees:

- **Equivalent:**
  - `if c { return true; } return false;` and `return c;` (for `c` = `temp[0] > 3`, `!(temp[0] > 3)`, `a > 3 && b == 2`, `self.attr[0] == 2`, `can_afford(..)`);
  - `if c { return false; } return true;` and `return !(c);`;
  - `if stack_obj.run_functional(4, change_icon: true) { return true; } return false;` and the lint's suggestion (a committed test).
- **Inconclusive:** `if c { return false; } return true;` and the flipped comparison (`return temp[0] <= 3;`), because the last result at exit differs.

## Decision 2: `lift --v2` writes the strings trees show

- **What is emitted.** The lift emits a `strings <id> "<label>" { name = "text", ... }` member for each string table that a tree shows text from:
  - Show String (21) and Set Action String (50) tables, by operand;
  - `STR#` 301 for a private Dialog (36) of an ordinary kind (0 to 4) that names a string. The picker and panel kinds (5 to 15) return before formatting any text.

  It writes a table only when the VM reads it as plain US English entries: one `STR#` chunk of that id, not deleted, named by the resource map, with resource flag `0x10`, a format from -3 to -1, a parse, and its language-1 entries in one run (or no entries at all). Any other table is left out with a `// strings <id> is not written: <why>.` comment, and its trees stay inconclusive. A table with no entries is written empty.
- **Entry names.** Each name is the snake case of the text's first four words. It falls back to `s<n>` and gets a `_<n>` suffix when two names would collide.
- **Tables left out.** A table is not written when a text holds a control character other than `\n`, `\r` or `\t`, which v2 string escapes cannot spell. Its trees stay inconclusive, and a comment in the output says why.
- **0-node trees.** A `BHAV` with no nodes lifts as `@frame(.., empty) tree x "x" {}`. `fmt` keeps both `@frame(empty)` and `@frame(args: 4, empty)` idempotent and compile-identical.
- **Lifecycle trees written plain.** An `init` or `main` tree that no lifecycle block reproduces (an empty one, or one with another header) has always lifted as a plain `tree`, and the build then gives the object its own lifecycle tree at a new ID. The tree's check covers its bytes, not the object's entry word. The lifted source now says so above the tree; the summary already counts these trees.

## Decision 3: PR G builtins in `lift --v2` (Sol, reviewed here)

The lift spells these as builtins instead of `raw {}`:

- `change_suit`, `burn`, `look_towards`, `drop_onto`, `find_location`;
- `relationship`, `old_relationship`, `generic_call`, `stop_all_sounds`;
- `job_data`, `distance_to`;
- `play_sound(flags:)`, `route_to(source:)`, `snap(mode:)`;
- the `sim.attr` / `sim.data` / `sim.container` / `sim.personality.*` places, and `handle.flags`.

Each spelling uses the per-tree proof and its raw fallback. These forms stay raw:

- operands v2 cannot spell: constant, temp or parameter `relationship` values, parameter-selected `drop_onto` slots, `play_sound` flag byte `0xFF`;
- rewrites that did not prove.

## Decision 4: returned tests lift as `return`

Two `if` shapes now lift as a single `return`:

| Lifted `if` | Becomes |
|---|---|
| `if a op b { return true; } else { return false; }` | `return a op b;` |
| `if a op b { return false; } else { return true; }` | `return !(a op b);` |
| `if prim(..) { return true; } else { return false; }` | `return prim(..);` |

All three lower to the same node with the same exits. A primitive test in the other polarity keeps its `if`. The lifted open pack, which lowers its guards this way, goes from 38 `redundant_bool_return` warnings to 0.

## Results (local only, aggregates)

- **Base game, 2722 trees** (`a9eacd99` rerun → this change):

  | Metric | `a9eacd99` | This change |
  |---|---|---|
  | Different | 0 | 0 |
  | Proven | 2376 | 2699 |
  | Inconclusive | 345 | 22 |
  | Omitted | 1 | 1 |
  | Idiomatic trees | 843 | 870 |
  | Raw statements | 1193 | 1119 |

  - Twenty of the trees still inconclusive show text from three tables whose resource lacks flag `0x10`. The VM's loader byte-swaps such a table's header, a path the port does not have, so the lift leaves them out (Decision 2) rather than write a table the compiler would mark `0x10`.
  - The other two are not about text. One is an animation event tree the verifier cannot decide (its paths may differ, but no replay confirmed it); the other is the tree that calls it.
- **Open pack, 116 trees:** 0 different. Proven goes from 114 to 116 (112 identical, 4 equivalent): the two dialog trees prove again. 112 are idiomatic.
- **Warnings per corpus, before → after:**

  | Corpus | Before | After |
  |---|---|---|
  | open-pack | 2 | 2 |
  | fridges pilot | 8 | 6 (two `negated_comparison` before a call) |
  | open-pack lifted | 2 | 0 |
  | base-game lifted | 48 | 29 |

  - The base-game lifted warnings left are 11 `magic_guid_literal`, 8 `raw_bare_call_false_exit`, 5 `redundant_bool_return` (`else if` arms and a `flag_test`), 3 `stack_obj_clobber_after_self` and 2 `frame_headroom`.
  - With the new lint and the old lift, the open-pack lifted corpus had 38 `redundant_bool_return` warnings; the return lift (Decision 4) clears them.

Every committed fixture is synthetic, with GUIDs in `0x7E00_xxxx`.
