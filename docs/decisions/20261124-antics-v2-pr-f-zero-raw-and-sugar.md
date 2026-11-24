# `.antics` v2 PR F: Last Lowering Losses, Last `raw` Gaps, and `if` / OR-Pattern Sugar

**Date:** 2026-11-24  
**Branch:** `antics/v2-pr-f-zero-raw-and-sugar`  
**Track:** Track 8 (`.antics` v2 Language Ergonomics & Extensions — PR F)

## Context

A third behavioural-equivalence pilot rewrote a 51-tree commercial object against PR E. As before, it ran locally and nothing from it is in the repository.

With `verify-equiv --behavioral`, 41 of its 51 trees prove one at a time. The other 10 stay INCONCLUSIVE, and because any opaque primitive may run any tree, those 10 cap the whole-file result. The causes are four v2 lowering losses and three `raw` gaps:

- **L7.** An entry `let __self_obj = stack_obj;` save that nothing needs. An advertisement write (`advert.delta[m] = v`, scope 12) counted as a `stack_obj` clobber.
- **L8.** `if obj.run_functional(..)` and `return obj.run_functional(..)` go through a `__cond` / `__ret` local and restore `stack_obj` before branching.
- **L9.** `for x in lot.objects()` always uses a cursor local, and `a || b` in an `if` always uses a `__cond` local. The original has a frame with no locals.
- **L10.** `push_interaction(object: x)` copies `x` into `local[0]` right before the push.
- **G1.** `inc_lt` / `dec_gt` cannot step the stack object's cells (`stack.data[0]`) in an object tree.
- **G2.** `push_interaction(icon:)` takes only an `i16` local, so one handle cannot supply both the object and its icon.
- **G3.** `animate` cannot send events to a local slot outside the tree's frame.

Track 9's decompiler also hit three base-game gaps:

- FWAV sound id 0;
- two FWAV ids that share one sound name;
- a BHAV `kind` byte.

The pilot also wanted `p.is_visitor` compared with an `i16` constant.

Constraints:

- **Same as PR E.** Everything lowers onto existing `.air`. Nothing changes in `src/{parser,lower,compile,verify*}.rs` (Track 7) or `src/v2/{fmt,decompile,lint}.rs` (Track 9).
- **Synthetic fixtures only.** Every test and example is synthetic.

## Decisions

### 1. Lowering losses

- **L7: advert writes are not clobbers, and an unread save goes.**
  - The `stack_obj` clobber analysis no longer counts:
    - `advert.delta | personality | minimum[m]`. These are scope 12 / 28 / 29, never an object access.
    - `stack_obj.<field>`. This is a direct `stack.` cell; see G1.
  - So that the analysis can trust the name, binding `stack_obj` as a variable, parameter, loop variable, or event name is now a parse error.
  - The optimizer forwards the entry save: until the first node that may write `stack_obj` (an assignment to it, or any primitive except `test_object_type`, `action_string`, and `alter_budget`; subroutine calls cannot), a read of `__self_obj` / `__saved_stack_obj` is `stack_obj` itself. A restore there becomes `stack_obj := stack_obj` and goes.
  - A save that nothing then names is dropped. It is the compiler's node, not the source's, so the tree's first statement runs on entry as written.
  - A `stack_obj := x` store goes when nothing can read the stack object before it is overwritten or the frame returns, on every path (a loop, a call, or a `stack.` read counts as a read), and the next node ignores the last result.
  - A tree that keeps its layout (`raw`) skips the optimizer. Even there, `finish()` drops the entry save when nothing else names `__self_obj` and the next node ignores the last result.
- **L8: `run_functional` branches directly.**
  - `if [!]obj.run_functional(..) { A } else { B }` lowers to `if [!]run_functional_tree(..) { restore; A } else { restore; B }`.
  - `return [!]obj.run_functional(..)` lowers to `if [!]prim { restore; return true; } restore; return false;`. This applies when no cleanup is active and the tree has no value-return ABI.
  - The optimizer then drops each `stack_obj` restore that is directly followed by `return`, including one at the end of an `if` arm when a `return` follows the `if`. An `else` left empty is removed.
- **L9: zero-local loops and short-circuit conditions.**
  - `for x in lot.objects()`, `lot.sims()`, and `objects_by_guid(G)` use `stack_obj` itself as the `set_to_next` cursor (`stack_obj := 0; while set_to_next(.., dest: stack_obj)`) when the body passes a conservative syntactic whitelist. The whitelist allows:
    - `if` / `return` / `break` / `continue`;
    - `let`s and assignments to plain locals, temps, and params;
    - comparisons and arithmetic over constants, locals, temps, params, `sim.` cells, BCON entries, and the cursor's own fields (`x.data[i]`, `x.attr[i]`, `x.person[i]`, `x.kind`, ...);
    - `x.is_type(..)`;
    - in object trees, `self.` fields (they are `my.`).

    Anything else keeps the cursor local, because it might read the stack object implicitly: calls, raw, `self` in a Sim tree, `stack_obj`, or any other handle's fields. Cursor field reads are direct `stack.` cells. The loop is followed by the usual restore, which the optimizer drops before a `return`.
  - **Flat `||` / `&&` in `if` conditions** lower without a `__cond` local, through a condition-free single-pass loop (`while true` emits no node):
    - `if a || b { T } else { E }` becomes `while true { if !a { if !b { E; break; } } T; break; }`.
    - `if a && b { T } else { E }` becomes `while true { if a { if b { T; break; } } E; break; }`.
    - `if a && b { T }` without `else` becomes `if a { if b { T } }`.

    Each test stays in evaluation order, so short-circuiting is unchanged. This applies only when every operand is a simple condition and neither body can `break` or `continue` (or `return`, inside an inlined helper) at the enclosing loop level. Otherwise the `__cond` local stays.
- **L10: the pushed handle lives in `local[0]`.** After optimization, a pinned tree whose every `local[0] := s;` copies the same plain local `s` renames `s` to `local[0]` and deletes the copies. Its `let s = e;` becomes `local[0] := e;`. The rename is sound because:
  - `.air` places named locals after `local[0]`;
  - a pinned tree has no `raw`;
  - only the push reads the slot;
  - `push_interaction` does not read the last result.

### 2. `raw` gaps

- **G1.** `stack_obj.<field>` names the stack object's cells directly in any tree:
  - `stack_obj.data[i]`, `stack_obj.attr[i]`, `stack_obj.graphic`, and the handle fields (`hand`, `room`, `person[i]`, ...);
  - there is no `stack_obj` switch around them.

  These cells, and `self.graphic`, are direct places for `inc_lt` / `dec_gt` / `random_into`. A foreign cell read on `stack_obj` is now the cell itself rather than a copy, and is a plain operand, so `while inc_lt(&mut stack_obj.attr[i], n)` is one node.
- **G2.** `push_interaction(icon:)` also takes an object handle or `self`, besides an `i16` local.
  - When it is the same handle as `object:`, the icon reads the pinned `local[0]` (flags `0x01 | 0x02`), so one handle supplies both.
  - Another handle must be a plain local.
- **G3.** `animate(.., event_local: N)` sends events to slot `N` (flag `0x20`) without binding them. It needs `@frame(locals: K)` with `N >= K`, so the slot is outside the frame and no binding can be there. After lowering, the tree's own locals are counted too: if they need more than `N` slots, it is an error. It lowers to the primitive's raw `op 0x002c (..)` form, because `.air`'s `animate(local: N)` would grow the frame to `N + 1`. It cannot be combined with `local:` or an `|event|` body.

### 3. Decompiler gaps

- **`sound x = preset(.., id: 0)`** is accepted.
- **Shared sound names.** Several `sound` declarations with distinct explicit ids may share one `wav:` / `name:` string; their presets must match.
  - `compile.rs` rejects duplicate manifest names, so v2 gives each extra declaration a unique placeholder name, `v2-shared-sound-{id}`. A sound may not use that name itself.
  - After compiling, it rewrites that FWAV chunk's sound name and label back to the shared name and drops the placeholder's synthesized WAV.
- **`@frame(kind: N)`** sets the BHAV header's kind byte (`.air` `kind:`).
- **`p.visitor_kind`** (and `sim.visitor_kind`) reads person word 32 as `i16`, so it compares with BCON kinds. `p.is_visitor` stays `bool`.

### 4. Sugar

- **`if` expressions.**
  - Syntax: `if c { a } else if d { b } else { e }` anywhere an expression may appear. `else` is required, and each branch is one expression.
  - New AST variant: `V2Expr::If { span, cond, then_value, else_value }`.
  - The branches must have identical types, as `match` arms must.
  - A constant condition folds, and the untaken branch is still checked.
  - `let x = if ..` writes `x` directly. Any other use writes a fresh local.
  - Conditions lower through the same simple, short-circuit, or `__cond` path as the `if` statement.
- **OR-patterns.**
  - Syntax: `A | B => ..` and `0 | 1 | 5..=9 => ..` in statement and expression `match`.
  - New AST variant: `MatchPattern::Or(Span, Vec<MatchPattern>)`. It is flat, and `_` inside one is an error. Identifiers stay, because `match dialog(..)` names its buttons (`Yes | Cancel`). On an `i16` or enum scrutinee an identifier is the usual error.
  - Every alternative takes part in duplicate and overlap checks, enum and `i16` exhaustiveness, and constant folding.
  - An arm whose alternatives are single compares (values, or `..=hi` / `lo..=` ranges) lowers with the single-pass-loop form, when its body and every later arm cannot `break` or `continue`: `while true { if !a { if !b { <later arms>; break; } } <body>; break; }`. Otherwise one flag local is set by the alternatives' tests (all are pure: the scrutinee is a plain operand or a snapshot), as a bounded range already was.
- **Allocation trims** in `lower.rs` and `opt.rs`:
  - the optimizer's identifier counts borrow the tree text instead of copying each name into a `String`;
  - "is this name mentioned" is a scan, not a count map;
  - the match chain reuses one buffer for each arm's tests.

## Consequences

- **Track 9 sees new AST variants:** `V2Expr::If`, `MatchPattern::Or`, and `TreeFrame::kind`.
- **Track 9's decompiler can now print:** `kind:`, sound id 0, shared sound names, `visitor_kind`, `stack_obj.<field>`, `push_interaction(object: x, icon: x)`, and `animate(event_local:)`.
- **Binding `stack_obj` as a name is now an error.** No in-tree source does it.
- **Zero-local loops change what the decompiler sees.** A synthetic `for o in lot.objects()` whose body passes the whitelist now compiles to the `dest: stack_obj` cursor. Track 9's decompiler prints it back as `stack_obj = 0; while set_to_next(..)`, which still proves equivalent. Lifting that shape to `for` is a decompiler follow-up; originals use it too.
- **Verifier gap (Track 7).** `set_to_next(.., dest: stack_obj)` and the default form (no destination, which is the stack object) behave the same but encode differently, and `verify-equiv` treats them as different primitives. Originals use both encodings for the same loop shape, so v2 keeps the explicit form. The verifier should treat them as one primitive.
