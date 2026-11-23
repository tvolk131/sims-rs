# `.air` primitive handling: one-table layout rejected, per-stage duplication removed

- **Date:** 2026-11-23
- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/parser.rs` (`parse_args`), `structure_lift.rs` (`DecodedIns::always_true` / `branching`, relaxed decoding), `verify.rs` (`canonicalize_primitive_operands` becomes `pub(crate)`)

## Context

The roadmap queued a one-table instruction-layout refactor of `sims-antics` after #402 and #406. The aim was to describe each of the 49 SimAntics primitives' operand layout once, instead of in every stage that handles it. It was to merge only if clearly shorter and cleaner, with every round-trip and equivalence verdict unchanged.

The audit found these per-opcode copies:

| Stage | Function | Works on |
| --- | --- | --- |
| Parse | `parser.rs::parse_primitive_call` (1,912 lines) | text → `PrimitiveCall` |
| Encode | `lower.rs::encode_primitive`, `scan_primitive` | `PrimitiveCall` → bytes, param/local usage |
| Decode / print | `structure_lift.rs::try_decode_instruction` | bytes → text, strict padding, relaxed masks |
| Verify | `verify.rs::canonicalize_primitive_operands`, `primitive_frame_uses`, safety `check_operand` arms | bytes |

## Why not one table

A declarative table would have to express, per field:

- operands that exist only under a flag bit or mode (about 20 of the 49 primitives);
- the bits the VM ignores (relaxed normalization, about 10);
- what strict lifting refuses because the AST cannot spell it (`remove: bool`, the dialog kind bit 7);
- positional-versus-named printing and omitted defaults, which differ per primitive and must stay byte-identical for the corpus round-trip;
- symbolic argument spellings (`tutorial(take)`, `look_towards(camera)`, refresh targets, route sources);
- implicit `param[0]` / local usage.

Some of the copies also disagree on purpose. For Animate, `verify.rs` treats any foreground clip that can emit events as writing `param[0]`, even with `events == 0`; this is a sound over-approximation. Lift and lower require `events != 0`. One table would either change a verdict or carry a per-consumer switch.

The table would therefore be a small interpreter over a field DSL, harder to read than the `match` arms it replaces. It is rejected.

## Decision

Remove the duplication that is mechanical, without changing any behaviour:

1. **`Parser::parse_args`.** 22 primitives repeated the same 40-line named/positional argument loop. Each now passes its positional key order and one `match key` closure. A script made the conversion and checked that every positional arm was textually identical to the named arm it maps to. The only exception is `flags =` against named `flags |=` in `burn`, `ui_effect` and `dialog`, which is equivalent because `flags` starts at 0 and no earlier positional slot sets it. `parse_primitive_call` drops from 1,912 to 1,176 lines.

   Nine loops with different grammar stay hand-written, because converting them would change which malformed inputs are accepted:
   - a required first argument followed by repeatable positionals, with no trailing comma: `get_distance_to`, `get_direction_to`, `find_best_object`, `remove_object`, `alter_budget`, `set_to_next`, `create_object`;
   - positionals allowed after named arguments: `animate`, `route_to`.
2. **`DecodedIns::always_true(head)` / `branching(expr)`** replace 53 hand-built struct literals.
3. **Relaxed lifting reuses `canonicalize_primitive_operands`.** `try_decode_instruction` canonicalizes the operands once when `strict_padding` is off, so the `if strict { b } else { b & mask }` sites in 12 opcodes (9, 11, 12, 13, 16, 23, 24, 26, 27, 34, 45, 46) are gone. Before this, the lifter and the equivalence checker each kept their own copy of "which bits the VM reads", and those copies could drift. Now there is one. Strict refusal (padding the AST cannot spell) stays per opcode, because it describes the AST, not the VM.

The one diagnostic change: `dialog(icon_mode: 9)` and `dialog(temp: 9)` errors now point at the out-of-range value instead of the key.

## Results

| | `origin/main` | this change |
| --- | --- | --- |
| `parser.rs` | 3,469 | 2,781 |
| `structure_lift.rs` | 3,960 | 3,743 |
| Net (`git diff --stat`) | | +421 / −1,326 (**−905**) |

Verdicts unchanged: see the PR body for the gate outputs.

## Not done (candidates)

- `verify.rs` safety `check_operand` arms and `primitive_frame_uses` overlap heavily. Unifying them changes safety verdicts in the Relationship write-back and Animate cases, so it needs its own review.
- `lower.rs::scan_primitive` runs before named locals are resolved, so it cannot reuse a byte-level table without a second encoding pass.
