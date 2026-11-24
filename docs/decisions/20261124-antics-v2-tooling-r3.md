# `.antics` v2 tooling round 3: `fmt`/`lint` for PR H, and PR H idioms in `lift --v2`

*Decided 2026-11-24.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/fmt.rs`, `crates/sims-antics/src/v2/lint.rs`, `crates/sims-antics/src/v2/decompile.rs`
- **Track:** Track 9 (round 3)

## Context

PR H (#440) added source the formatter, linter and decompiler did not know yet:

- `@slot(N)` locals;
- `@context(inherit)` trees;
- `const` bindings by `if`/`match` (branch specialization);
- `@frame` on lifecycle blocks;
- `set_to_next(.., dest: stack_obj)`.

Three problems followed from that:

- `fmt` printed `@ slot(3)` and `const(A, B)`, and split a one-line `const X = if c { a } else { b };` over five lines.
- `lint` misreported two of the new forms:
  - A `const` named only inside `raw { }` was reported as unused, with advice that is no longer true: raw does resolve v2 consts since PR H.
  - A pinned `@slot(N) let x: T;` (read by number by a raw primitive) was reported as never read or as read uninitialized.
- `lift --v2` wrote none of the new forms, so it reached for `raw` (or a plain tree) where PR H now has a word.

The compiler, verifier and CLI belong to other tracks and do not change here.

## Decision 1: `fmt`

- **Attributes:** `@slot` and `@context` print tight, like `@id`/`@frame`/`@guard_id`:
  - `@slot(3) let x: i16;`
  - `@id(4300) @context(inherit)`, with `tree` on the next line as written.
- **`const`:** spaced as a keyword: `const (A, B) = ..`.
- **One-line bindings:** a `const NAME = if c { a } else { b };` written on one line stays on one line, when:
  - there is no comment inside;
  - there is no `raw`;
  - each arm is one value (no `;`, `{` or `}` inside).

  Anything else is exploded as before.

Checked over 689 pilot `.antics` files:

- 175 changed, on attribute and `const` lines only;
- 0 were not idempotent;
- every file that expands without a pack (159) expands to the same `.air`.

## Decision 2: `lint`

- **`unused_const`:** a `const` named in a `raw { }` block counts as used, and the "raw does not resolve const" advice is gone.
  - A raw block's own `.air` `const K = ..;` declarations are not uses of a v2 `const K`.
- **`@slot(N) let`:** a pinned local (with or without an initializer) is neither `unused_decl` nor `uninitialized_local_read`, since a raw primitive or text reads it by number.

## Decision 3: `lift --v2` writes the PR H idioms where they are proven

Every idiom stays behind the decompiler's existing proof loop. A tree whose recompile is not proven `IDENTICAL`/`EQUIVALENT` drops the idiom, then lowers.

1. **Lifecycle blocks keep their header.**
   - An `init`/`main`/... block the default header does not reproduce is written `@id(N) @frame(args, locals, format, revision[, kind]) main {`, instead of a plain tree plus a generated stub. A block whose default header is proven is still written bare.
   - Order of adding `@frame`:
     - Formats other than 0x8002, and nonzero kinds, start framed.
     - Any other block gets `@frame` before anything else is tried, when it fails to compile (for example `param[0]` outside a zero-argument frame) or is not proven.
     - A round that adds a frame lowers nothing else, because a caller is proven only together with its callees.
   - Only a 0-node lifecycle tree still starts plain: `@frame(empty)` is refused on blocks.
   - **Consequence:** a call to a lifecycle block (`call 4101(temps)`) stays `raw`, because a block has no callable name. This is more faithful than the old plain tree plus stub.
2. **`return error;`** is written for an error exit outside a handler.
3. **`set_to_next(word[, GUID][, dest: stack_obj][, local: x])`** replaces the raw primitive for the modes PR H names:
   - all, person, non_person, part, guid, neighbor, category, neighbor_guid, on_tile, beside, career, family;
   - `local:` only for `beside`.

   It is used outside cursor loops only. A GUID literal over 0xFFFF, or one used by more than one primitive, gets a named `const`, so no `magic_guid_literal` is introduced.
4. **`@slot(N) let slot_N: i16;`** replaces the `raw { /* Preserve builtin local slot numbers. */ }` plus `let` pair for the builtin locals a primitive names by number.
5. **`const` ladders.** A plain `if c { A } else { B }` (no `else if`) is rewritten when:
   - its arms are token-for-token equal except for 1–4 integer literals;
   - there are no `raw`, attributes, `let`s or blank lines.

   The rewrite is `const ARM_n = if c { a } else { b };` (or the tuple form), followed by the arm once with the literals named.
   - The last arm statement must use a binding. PR H's specialization region ends at the last statement naming a bound name, so a binding used only earlier would leave that tail outside the region.
6. **`@context(inherit)` + `me.*`.** A private, non-lifecycle tree called from a tree of the other context (by the contexts the trees prefer) is written `@context(inherit)` when idiomatic:
   - its own-object scopes print as `me.attr/data/motive/person/slot[i]`;
   - stack-object scopes print as `stack_obj.*`.
7. **Argless calls between Sim trees** now pass `sim`, like calls with temps already did.

Not lifted: the rare raw `sleep` statements.

## Results (local corpus gate; generic metrics only)

**Base game:** 2722 trees across 126 objects.

| | main (b3758a7c) | this change |
|---|---|---|
| proven (identical + equivalent) | 2699 (367 + 2332) | 2699 (367 + 2332) |
| inconclusive / omitted / DIFFERENT | 22 / 1 / **0** | 22 / 1 / **0** |
| idiomatic trees | 870 | 891 |
| raw statements inside idiomatic trees | 1119 | 769 (−31%) |
| raw trees | 1851 | 1830 |
| init/main written as plain trees | 101 | 0 |
| lint warnings on the lifted files | 29 | 17 (`magic_guid_literal` 11 → 0) |

- **Open Starter Pack:** 116/116 trees proven, 0 DIFFERENT. The lifted files lint clean and pass `fmt --check`, and are byte-identical to main's lift.
- **`fmt --check open-pack`:** clean. `lint open-pack` reports the same 2 `unused_decl` warnings as on main (`state reserved` in two objects). Those files are outside this track.
- **Fridges pilot lint:** unchanged.

## Limits

- **Verifier limits** (Track 7, out of scope here) keep idiomatic trees from rising further. The remaining raw trees fail mostly on three:
  - a loop not bounded by literals;
  - an opcode that runs at a different node in the two trees, where a fault observes the node;
  - the path cap.
- A single-value ladder needs its binding in the arm's last statement (see Decision 3.5).
- A block call stays `raw` (see Decision 3.1).
