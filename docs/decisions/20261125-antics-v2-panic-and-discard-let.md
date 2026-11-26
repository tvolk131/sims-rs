# `.antics` v2: `panic!()` and `let _ = expr;`

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/{ast,parser,fmt}.rs` and `src/v2/lower/{stmt,mod}.rs`.
  No change to `v2/decompile.rs` or `v2/lint.rs`.
- **Track:** Track 8 (compiler), `.antics` v2.1 Rust-idiom polish
- **Builds on:** [`20261125-antics-v2-rust-like-syntax.md`](20261125-antics-v2-rust-like-syntax.md)

## Context

Two everyday Rust spellings had no `.antics` form:

- **The error exit.** SimAntics has one: return target 253, which carries no message. The
  language spells it `return error;`. That reads like returning a value named `error`, and it
  does exactly that when a binding named `error` is in scope.
- **Discarding a result.** A `Result` call as a bare statement is refused ("unused Result").
  `let _ = f();` was accepted, but it made a local named `_` and stored the result in it.

## Decision 1: `panic!()` is the error exit

- `panic!()` parses to the existing `return error;` statement (`V2Stmt::ReturnError`), with
  a new `panic: true` flag. It lowers to the same `.air` and the same `.iff` bytes.
- It is accepted wherever `return error` is:
  - as a statement, `panic!();`;
  - as a block's last statement without `;` (`if c { panic!() }`);
  - as a `match` arm, in both value and statement matches (`_ => panic!(),`);
  - as a diverging branch of a value `if` or `match` (`else { panic!() }`).
- Unlike a written `return error;`, it is always the error exit, even when a binding named
  `error` is in scope. `cond?;` already worked this way through `from_try`.
- It is refused:
  - inside `defer` (`'panic!()' cannot exit out of a 'defer' block`);
  - with arguments: the exit carries no message;
  - in value position (`let x = panic!();`): it is a statement or a diverging branch.
- `return error;` stays valid. Moving the decompiler and lints to `panic!()` is a separate
  step, owned by Tracks 9 and 7.
- The formatter prints `ident!(` with no space (`panic!()`). The rule holds only when `!` is
  directly followed by `(` after a non-keyword identifier. Prefix `!` never follows an
  identifier, so it keeps its spacing.

## Decision 2: `let _ = expr;` is `expr;`, or `if expr {}` where the statement is refused

The rule: `let _ = expr;` does what the statement `expr;` does when that statement exists and
means the same. Otherwise it does what `if expr {}` or an unused `let y = expr;` does. The
binding `_` is never bound. Lowering:

- **`expr?`** lowers exactly as the statement `expr?;`. On a `Result<bool>`, the payload is not
  tested.
- **A call**, resolved in the order the statement `expr;` dispatches it:
  - **Inline helper:** a helper returning bool or `Result` lowers exactly as `if expr {}`,
    with no return local. Any other helper lowers as `expr;`. The choice comes from the
    declared return type, so a chain of helpers that discard each other is lowered once per
    level, not once per trial.
  - **`#[bhav]` tree, `extern mod` function or `run_functional`:** lowers exactly as
    `if expr {}`, the same nodes as the statement, which refuses an unused `Result`. A
    `Result<i16>` value falls back to a hidden local.
  - **Builtin:** lowers exactly as its statement `expr;`, which can differ from its
    condition: the legacy `create_object` restores the stack object. Without a statement
    form, it lowers as `if expr {}`. Builtins dispatched before helpers (`sleep`,
    `play_sound`, `dialog`, and so on) win over a helper with the same name, as in the
    statement.
  - **`let _ = animate(..);`** is parsed as the statement `animate(..);`, which waits for
    the clip. A parenthesized `(animate(..))` is the one-node value, as the statement
    `(animate(..));` is.
  - Errors are the statement's, except when it refuses only an unused `Result`. Then the
    `if`'s error is reported.
- **Any other bool expression** lowers exactly as `if expr {}`.
- **A literal** is checked, then dropped. An unused `let y = 1;` leaves no node either.
- **Anything else** is an unused `let y = expr;` of a hidden, unbound local. The optimizer
  keeps exactly what it keeps for one. Even a node over locals moves the scratch pointer
  that `global[34]` reads, and a cell read may fault or wake a VM wait. So a value is never
  dropped where the named form would keep it. This is why `let _ = x + 1;` keeps its nodes.

The parser refuses the forms that would bind something:

- `let mut _`;
- `let _: T`;
- an uninitialized `let _;`;
- `#[slot(N)] let _`;
- `_` in any expression position.

The last rule also refuses reading the binding of `for _ in a..b` (`self.n = _;`), which
compiled before. Rust refuses it too, and nothing in open-pack did it.

A trial lowering copies the whole lowerer, emission included. A failed trial leaves no state,
and a kept one keeps the offsets its statement marks record. `let _` records no declaration
unless it falls back to a hidden local. So a region that a `const` copies may hold a
`let _` of a call or a literal, even in a tree that formats `$Local:N`.

## Consequences

- No `lint.rs` or `decompile.rs` edit was needed. Their `ReturnError` patterns use `..`.
- Some of their messages still say "`return error`" for a `panic!()` (unreachable-code).
  Tracks 7 and 9 own that wording when they adopt `panic!()`.
- `lower_let` is now a method of its own, shared by the `let` arm and the discard fallback.
- `STMT_BUILTINS_BEFORE_HELPERS` in `lower/stmt.rs` mirrors the builtins `lower_expr_stmt`
  dispatches before helpers. A new statement builtin there must be added to it too.
- `InheritCallFacts` derives `Clone`, so a failed trial lowering restores it along with the
  dialog string table.
- open-pack: byte-identical, lint and `fmt` clean.
