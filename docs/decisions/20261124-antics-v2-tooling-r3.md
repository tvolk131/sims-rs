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

The code check compares a tree's frame and code. It does not compare the object data the game reads around them, so the lift now checks that data itself:

- **The revision** (saved objects are matched against it). A mismatch leaves the tree unproven (`header_gap`), and `@frame` fixes it.
- **The label** (Run Tree By Name finds a tree by it). A rebuilt tree labelled differently is reported inconclusive (`Model::object_gap`).
  - Labels are written verbatim and escaped: quotes, backslashes, Windows-1252 letters and trailing or all-blank spaces are kept.
  - A label the compiler cannot store (a control character, a character outside Windows-1252, more than 63 bytes) is cleaned and cut to fit. The lift goes on and that tree is reported inconclusive.
- **The trees a primitive finds by name.**
  - Run Tree By Name reads its name from `STR#` 303. The lift now writes that table; before, it dropped it.
  - Create Object Instance (with its from-record flag), Go To Relative Position and Go To Routing Slot (which share one routing state machine), and Reach look up fixed names (`init visitor`, `route ask to move animation`, `route waiting animation`, `Route failure feedback`, `CT - Grab`, `CT - Put`).
  - The labels are the VM's: the last BHAV chunk of each ID, deleted ones included.
  - The VM searches the owner's file, its semi-global file and the global file by exact name, then all three again ignoring case. Which file is the owner varies, so a name is checked in this file whichever it is:
    - the exact pass in this file must agree;
    - when it finds nothing, the `GLOB` link and the case-ignoring pass must agree too.
  - A name not known here (from the global file's table) needs the whole label map and `GLOB` link to agree. This file may itself be the global file, so its own `STR#` 303 entry must not change either. Entry 0 names nothing in either table.
  - A tree a name finds is a callee: a caller is proven only with it (every tree of the file, for a name not known here).
- **The object's callbacks.** For every object definition (by GUID), the trees its callback slots run, the VM's way, conditions included:
  - the `OBJf` table when there is one, else the definition's own words;
  - one table per OBJD chunk ID, the last definition of that ID winning, shared by every definition with that ID.

  A tree registered in different slots in the rebuilt object is inconclusive. This catches:
  - a plain tree written for `init`/`main`, whose entry then runs a compiler-made block;
  - a tree shared by two slots;
  - an `OBJf` override;
  - a definition the lift drops.
- **Run Functional Tree** runs the callback its function names, on the stack object. The action every definition has there must agree, and is a callee of the caller.
- **Create Object Instance** runs the new object's callbacks (`init`, then a turn of `main`), with or without its from-record flag. The new object may be this one, so every callback table must agree, and every callback tree is a callee.
- **Definition and callback reads.** An Expression reading scope 21 (the stack object's definition words) or scope 35 (whether its callback table names a tree) needs this file's definitions (the live ones, in resource order, byte for byte), or its callback tables, to agree.
- **The semi-global link.** Semi-global calls and constants go through `GLOB`, and so does every global tree a tree calls, since it runs with this object as its owner. When the rebuilt object's link differs (an empty name links nothing), no tree of the object is proven.
- **Non-private code of this file.** The lift writes only private trees (4096–8191) and private constant tables (4096–4159). A file holding other trees or constant tables (for example the global file, or a file other objects link as their semi-global) is a file whose trees reach them by number, by a name another file supplies, or through constant addresses. When the rebuilt file does not keep them, no tree of the object is proven.

**File roles.** The checks hold whichever role the lifted file plays for a running tree: the object's own file, another object's semi-global file, or the global file.

**What the per-object note covers.** The lift's existing note lists the resources the rebuilt object does not reproduce, as the VM reads them (live resources in resource order; for trees, the last chunk of each ID). The list covers:
- `STR#`, `CTSS`, `TTAB`, `TTAs`, `SLOT`, `FCNS`, `GLOB`, `TPRP`, `OBJD` and `OBJf`;
- non-private BHAVs and BCONs.

Some of these also have the per-tree checks above (`STR#` text and names, `OBJD`/`OBJf` callbacks and Expression reads, `GLOB`, non-private code). For those, the note is informational: a tree that depends on them in a way the checks see is already inconclusive. The rest are read only inside primitives (routing slots, interaction tables, Test Object Type, ...) or by trees of other objects reading this one, and the tree verdicts treat those reads as the same in both files. Chunk types outside the list (graphics and other assets) are neither listed nor compared: primitives that read them are treated as reading the same data in both files. The CLI's wording ("trees that read them are checked as code only") is in `src/bin/sims-antics.rs`, outside this track.

These gaps are settled: no other spelling of the tree closes them, so the tree is not lowered for them. A caller of an unproven tree stays unproven as before.

They also close soundness gaps on main, which reported these trees proven:

- every lifecycle block (named after its kind, `Main`/`Init`);
- every plain-tree `init`/`main` (whose entry ran a generated stub);
- every Run Tree By Name (whose rebuilt object had no names table);
- every tree of an object with a semi-global file: v2 cannot write `GLOB`, so the lifted object has none;
- every callback the lifted object does not register: v2 writes `init`/`main` (and a catalog variant's), `load`/`reset`/`cleanup` and a fixed set of `functional` kinds, each with a compiler-chosen label. The lift writes none of the other OBJD callback words, no `functional`, and no definition it cannot write as a variant.

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
| proven (identical + equivalent) | 2699 (367 + 2332)* | 1555 (218 + 1337) |
| inconclusive / omitted / DIFFERENT | 22 / 1 / **0** | 1166 / 1 / **0** |
| idiomatic trees | 870 | 891 |
| raw statements inside idiomatic trees | 1119 | 768 (−31%) |
| raw trees | 1851 | 1830 |
| init/main written as plain trees | 101 | 0 |
| lint warnings on the lifted files | 29 | 17 (`magic_guid_literal` 11 → 0) |

\* Main's count is not sound. It does not check labels, callbacks, tree names or the semi-global link (see *Object-level checks*).

The 1166 inconclusive trees, by the first reason reported:

- 371 callbacks the lifted object does not register (see above);
- 354 trees of the 36 objects with a semi-global file (the lift writes no `GLOB`);
- 219 trees that call an unproven tree, directly, by name, by function or by creating an object;
- 103 lifecycle trees whose label a block cannot keep;
- 53 trees that create an object while the callback tables differ;
- 23 trees looking a tree up by a name not known here (mostly the global file's table);
- 18 Run Functional Tree callers whose callback differs;
- 15 readers of definition words and 6 of callback tables that differ;
- 4 trees that read a `STR#` table the lift does not reproduce.

Writing `STR#` 303 recovered 187 by-name trees and callers that the checks alone would also have left unproven.

- **Open Starter Pack:** 110/116 trees proven, 0 DIFFERENT. The lifted files lint clean and pass `fmt --check`, and are byte-identical to main's lift. The 6 inconclusive trees are real gaps main reported proven:
  - the stove's and counter's `functional` guard and action, whose OBJf registration the lift drops;
  - the double bed's `init`/`main` for its two extra tile definitions, which the lift drops.
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
- **Callbacks and the semi-global link** (most of the 1166 base-game trees, all 6 open-pack trees) need follow-ups:
  - lift `functional`, `load`/`reset`/`cleanup` and multi-tile definitions (Track 9);
  - a v2 way to fill any OBJD callback word with a labelled tree, and to write the `GLOB` link (Track 8).
- `@slot(N) let x: T;` is exempt from `uninitialized_local_read` as a whole. A pinned slot exists so raw code or text can use it by number, and lint does not model raw writes by slot number.
