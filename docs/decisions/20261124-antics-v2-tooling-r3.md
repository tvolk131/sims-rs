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
  - A raw block's own `.air` `const K = ..;` declares that block's `K`, so a bare `K` in the block is not a use of a v2 `const K`.
  - `const[K]` reads `K`, and `x.const` declares nothing.
  - `const K.e = ..` declares the block's `K.e`: a `K.e` in that block is not the v2 `K`, but a bare `K` still is.
  - A field after `.` (`my.room`) is not a use of a v2 `const room`.
- **`@slot(N) let`:** a pinned local (with or without an initializer) is neither `unused_decl` nor `uninitialized_local_read`, since a raw primitive or text reads it by number.

## Decision 3: `lift --v2` writes the PR H idioms where they are proven

Every idiom stays behind the decompiler's existing proof loop. A tree whose recompile is not proven `IDENTICAL`/`EQUIVALENT` drops the idiom, then lowers.

### Object-level checks the code check does not make

The code check compares a tree's frame and code. It does not compare four things the game reads, so the lift now checks them itself:

- **The revision** (saved objects are matched against it). A mismatch leaves the tree unproven (`header_gap`), and `@frame` fixes it.
- **The label** (Run Tree By Name finds a tree by it). A rebuilt tree labelled differently is reported inconclusive (`Model::object_gap`).
  - Labels are written verbatim and escaped: quotes, backslashes, Windows-1252 letters and trailing or all-blank spaces are kept.
  - A label the compiler cannot store (a control character, a character outside Windows-1252, more than 63 bytes) is cleaned and cut to fit. The lift goes on and that tree is reported inconclusive.
- **The names a Run Tree By Name finds.**
  - The lift now writes the `STR#` 303 table such a call reads its name from; before, it dropped the table.
  - Each call naming an entry of this file's table is resolved in the original's and the rebuilt file's labels, the VM's way: exact name, then ignoring case, two matches an error. A different result leaves the caller inconclusive.
  - A name from the global file's table is not known here, so any label change in the file leaves that caller inconclusive.
- **The object's `init`/`main` entries.** A lifecycle tree the rebuilt object's entry does not run (a plain tree written for it gets a compiler-made block) is reported inconclusive.

These gaps are settled: no other spelling of the tree closes them, so the tree is not lowered for them. A caller of an unproven tree stays unproven as before.

They also close soundness gaps on main, which reported these trees proven:

- every lifecycle block (named after its kind, `Main`/`Init`);
- every plain-tree `init`/`main` (whose entry ran a generated stub);
- every Run Tree By Name (whose rebuilt object had no names table).

v2 has no way to label a lifecycle block, so a block labelled e.g. `main loop` cannot be proven. A label on blocks would be a compiler change (Track 8).

1. **Lifecycle blocks keep their header.**
   - An `init`/`main`/... block the default header does not reproduce is written `@id(N) @frame(args, locals, format, revision[, kind]) main {`, instead of a plain tree plus a generated stub. A block whose default header is proven is still written bare.
   - A block whose label the compiler changes (`Tick` written as `main` becomes `Main`) stays a block and is reported inconclusive. A plain tree would keep the label, but the object's `main` entry would then run a compiler-made stub.
   - Order of adding `@frame`:
     - Formats other than 0x8002, nonzero kinds and nonzero revisions start framed.
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

   A literal's unary `-` belongs to it, so `-5`/`-6` arms lift too. Arms of opposite sign differ by a token and stay an `if`.

   The rewrite is `const ARM_n = if c { a } else { b };` (or the tuple form), followed by the arm once with the literals named.
   - The last arm statement must use a binding. PR H's specialization region ends at the last statement naming a bound name, so a binding used only earlier would leave that tail outside the region.
6. **`@context(inherit)` + `me.*`.** A private, non-lifecycle tree called from a tree of the other context (by the contexts the trees prefer) is written `@context(inherit)` when idiomatic:
   - its own-object scopes print as `me.attr/data/motive/person/slot[i]`;
   - stack-object scopes print as `stack_obj.*`, including `stack_obj.person[65]` outside a `for p in lot.sims()` loop.
7. **Argless calls between Sim trees** now pass `sim`, like calls with temps already did.

Not lifted: the rare raw `sleep` statements.

## Results (local corpus gate; generic metrics only)

**Base game:** 2722 trees across 126 objects.

| | main (b3758a7c) | this change |
|---|---|---|
| proven (identical + equivalent) | 2699 (367 + 2332)* | 2385 (342 + 2043) |
| inconclusive / omitted / DIFFERENT | 22 / 1 / **0** | 336 / 1 / **0** |
| idiomatic trees | 870 | 891 |
| raw statements inside idiomatic trees | 1119 | 768 (−31%) |
| raw trees | 1851 | 1830 |
| init/main written as plain trees | 101 | 0 |
| lint warnings on the lifted files | 29 | 17 (`magic_guid_literal` 11 → 0) |

\* Main's count is not sound. It does not check labels, entries or tree names (see *Object-level checks*).

The 336 inconclusive trees, by the first reason reported:

- 103 lifecycle trees whose label a block cannot keep;
- 47 Run Tree By Name callers whose name may resolve differently (mostly global-table names, not known here);
- 182 trees that call an unproven tree;
- 4 trees that read a `STR#` table the lift does not reproduce.

Writing `STR#` 303 recovered 187 by-name trees and callers that the checks alone would also have left unproven.

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
- 103 base-game lifecycle trees carry labels other than the compiler's block names (`main loop`, ...). They and their by-name callers stay inconclusive until v2 can label a block (Track 8 follow-up, e.g. `main "main loop" { }`).
- Run Tree By Name from the global file's names table is checked conservatively. Resolving it would need the global file's `STR#` 303 in the lift options.
- `@slot(N) let x: T;` is exempt from `uninitialized_local_read` as a whole. A pinned slot exists so raw code or text can use it by number, and lint does not model raw writes by slot number.
