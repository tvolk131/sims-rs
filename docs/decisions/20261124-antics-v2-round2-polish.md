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
| `negated_comparison` | Reported only when both successors of the `!(a op b)` test are **quiet**: a store to `temp[n]`, or to a field of `self` outside a Sim's body, with a value made of literals, locals, enum paths and such places (no division, no indexed read); or a test of such operands that cannot fold. A call, `sleep`, a primitive, `return`, a store to a `let` (which may fold away) and anything touching `self` in a Sim's body (where `self` follows `stack_obj`) are not quiet. |
| `redundant_bool_return` | Reported for `if c { return true; } return false;` (or with `else`), comparisons and `&&` included, with the suggestion `return c;`. The other polarity is never reported: `return !c;` ends on the opposite last result. |
| `uninitialized_local_read` | `relationship(value: x)` writes `x`, unless its `flags:` are a literal with flag 4 (which writes the relationship from the local). Flags that are not a literal count as a write. |
| `unused_decl` (const) | A `const` named only inside `raw {}` is reported, with a note: `raw` does not resolve `const` names, and the build rejects the name there. A value written in its place still counts as a use. |
| `unused_decl` (let) | A tree that shows strings this file does not hold is treated as possibly reading every local through `$Local:N`. This covers a dialog's `STR#` 301 number past the file's own entries (the object's spliced table), `show_string` and `action_string`. |
| directives | `// antics-lint: allow(..)` and `# antics-lint: allow(..)` work inside `raw {}`, per line: trailing a line covers that line, and on its own line covers the next line of code. Inside `raw`, `//!` is never file-level, and a `#` inside an `.air` string is not a comment. |

The verifier checked the `redundant_bool_return` change on synthetic trees:

- **Equivalent:**
  - `if c { return true; } return false;` and `return c;` (for `c` = `temp[0] > 3`, `!(temp[0] > 3)`, `a > 3 && b == 2`, `self.attr[0] == 2`, `can_afford(..)`);
  - `if c { return false; } return true;` and `return !(c);`.
- **Inconclusive:** `if c { return false; } return true;` and the flipped comparison (`return temp[0] <= 3;`), because the last result at exit differs.

## Decision 2: `lift --v2` writes the strings trees show

- **What is emitted.** The lift emits a `strings <id> "<label>" { name = "text", ... }` member for each string table that a tree shows text from:
  - Show String (21) and Set Action String (50) tables, by operand;
  - `STR#` 301 for a private Dialog (36) that names a string.

  It takes the last `STR#` chunk of each id and keeps its language-1 entries, which are what the VM reads.
- **Entry names.** Each name is the snake case of the text's first four words. It falls back to `s<n>` and gets a `_<n>` suffix when two names would collide.
- **Tables left out.** A table is not written when a text holds a control character other than `\n`, `\r` or `\t`, which v2 string escapes cannot spell. Its trees stay inconclusive, and a comment in the output says why.
- **0-node trees.** A `BHAV` with no nodes lifts as `@frame(.., empty) tree x "x" {}`. `fmt` keeps both `@frame(empty)` and `@frame(args: 4, empty)` idempotent and compile-identical.

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
  | Proven | 2376 | 2719 |
  | Inconclusive | 345 | 2 |
  | Omitted | 1 | 1 |
  | Idiomatic trees | 843 | 871 |
  | Raw statements | 1193 | 1120 |

  The two trees still inconclusive are not about text. One is an animation event tree the verifier cannot decide (its paths may differ, but no replay confirmed it); the other is the tree that calls it.
- **Open pack, 116 trees:** 0 different. Proven goes from 114 to 116: the two dialog trees prove again. 112 are idiomatic.
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
