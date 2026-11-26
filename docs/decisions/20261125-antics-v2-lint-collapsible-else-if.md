# `.antics` v2 lint: `collapsible_else_if`

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`, `sims-antics lint`
- **Track:** Track 7 (`.antics` v2.1 Rust-idiom polish, Step 1)
- **Builds on:** [20261125-antics-v2-lint-wave4.md](20261125-antics-v2-lint-wave4.md)

## Context

v2 has `else if`, but nothing asks for it. An `else` block that holds nothing but one `if` can
be written `else if ..`, as Clippy's `collapsible_else_if` says for Rust. The lift writes
the long form, and so do people porting nested `.air` trees by hand.

## Decision

A new warning, `collapsible_else_if`, reports `else { if .. }` where the block holds one `if`
(with any `else if` / `else` chain of its own) and nothing else. It covers statement `if`s
and `if` expressions (`let x = if a { 1 } else { if b { 2 } else { 3 } };`). It points at the
`else` and gives a help. It has no machine fix.

### How it tells the spellings apart

The parser reads `else { if c { .. } }` and `else if c { .. }` to the same tree: the `else`
branch is `[If]` both ways. Because the trees match, the collapse cannot change what is
built, and no build gate is needed (unlike wave 3's spellings). It also means the AST cannot
tell the two apart, so the rule reads the tokens around the inner `if`:

1. The tokens before the inner `if` must be `else` and `{`. `else if` has `else` right before
   it and is not reported.
2. A walk forward from the inner `if` over its condition, its block and its `else if` /
   `else` chain must end on the token just before the `}` that closes the `else` block. So a
   statement before or after the `if` (`let`, a block `const`, another call) stops the
   finding, whatever the AST shows after constant substitution.
3. The walk reads a condition up to the first `{` outside brackets. When the condition
   holds a brace inside brackets, or an `if` / `match` (whose braces the walk could misread),
   the `if` is skipped. This leaves a false negative, never a misread.

A `raw { }` body is one opaque token, so `.air` text that looks like v2 is never read, and an
`else` holding a raw block is no `if`. The parser takes `for .. else { .. }` and
`let Some(..) = .. else { .. }` as blocks only (`else if` does not parse there), so the rule
looks at `if` statements and expressions only. A malformed `if` does not parse, so the file
is not linted at all.

### Comments

The rule follows wave 3's rule for comments. The collapse drops text in three places:

- between `else` and `{`;
- between `{` and the inner `if`;
- between the end of the inner chain and the block's `}`.

The rule reports only when all three are whitespace. Tokens hold no comments, so any other
text there is one. Comments the collapse keeps do not stop it: comments inside the inner
`if`'s blocks, and comments before the `else`.

An `allow` written at the end of the `} else {` line sits between `{` and the `if`, so it
already stops the finding and then counts as stale. Write the directive on its own line
before `} else {` instead.

## Results

- **Open pack:** `lint --deny-warnings` finds nothing, and the build is byte-identical to
  main.
- **Lift corpus** (`cache/track9/lift-wave3-beh`, 1,191 behavioral lifts of main after
  #473, each file linted alone):
  - 0 errors or panics.
  - 630 `collapsible_else_if` findings in 280 files. Every other rule is unchanged from main.
  - With all 630 collapsed by a script, all 280 files still `check`, `expand` to the same
    `.air`, and lint with no `collapsible_else_if`.
- **Lift tests:** `lift_source` asserts that lifted text lints clean. One `decompile.rs` unit
  test, `behavioral_mode_proves_a_renumbered_tree_and_says_so`, lifts
  `} else { if !(..) { return true; } }`, which this rule reports. `decompile.rs` is Track 9's
  file, so it is not edited here: the lift should write `else if` for an `else` arm that holds
  only an `if`.

## Consequences

- Track 9's lift needs to write `else if ..` for an `else` arm that holds only an `if`. That
  clears the lifted corpus and the one unit test.
- A machine fix (delete the block's braces and dedent its body) could follow if the lift wants
  to apply it the way it applies wave 3's suggestions.
