# `.antics` v2 canonical formatter and high-level decompiler

*Decided 2026-11-23.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/{fmt,decompile}.rs`, `sims-antics fmt`, `sims-antics lift --v2`
- **Track:** Track 9

## Context

v2 sources are written by hand, and lifting a file only produced `.air`. Two tools were missing:

- a formatter, so hand-written and generated v2 share one layout;
- a decompiler that turns a compiled object back into readable v2 that compiles again.

Constraints:

- Nothing in the v2 lexer, parser, AST, loader or lowering may change (Track 8 owns them).
- Nothing in the verifier may change (Track 7 owns it).
- Every committed fixture is synthetic.

## Decision 1: the formatter reflows tokens, not the AST

`format_v2_source` does three things:

1. It parses the input with `parse_v2_file`, so invalid input is refused.
2. It scans the source into tokens plus trivia: comments, and how many line breaks came before each token.
3. It prints the tokens again with canonical whitespace.

It does not pretty-print the AST. The AST drops comments and sugar (`loop`, `cond?;`, `:=`, `0x7F00_0020`, meta field aliases), and printing it would mean re-spelling every construct in a 3,600-line grammar. Reflowing tokens keeps every token's text exactly, so:

- **Semantic preservation is checked, not hoped for.** After formatting, the output is lexed again with the real v2 lexer. Its token sequence must equal the input's, ignoring spans. `raw { ... }` bodies count as equal when their trimmed lines are equal, which is all lowering reads. Any difference is an error, never a silent rewrite.
- **Every comment survives:** `///`, `//`, trailing, and `/* */` (nested too). Comments are trivia and are printed verbatim.
- **Idempotence comes from the layout rules.** Each rule depends only on facts the output itself preserves.

Layout rules:

- 4-space indentation.
- A newline after `;` and after `{`, and before `}`. Commas inside a brace block (enum/bitflags/bcon entries, interaction fields, match arms) also end the line.
- `} else`, `};`, `},` and `})` stay joined.
- A `(...)` or `[...]` group stays on one line, unless the source broke a line inside it, or it holds a comment or a non-empty brace block. Then it is laid out vertically: one element per line, indented once.
- `use a::{B, C}` braces stay inline.
- Expression `match` arms and `variant "Name" { ... }` fields are brace-block entries like any other: one per line, with `};` joined after an expression `match`. `variant` is a declaration keyword for blank-line grouping.
- Operators are spaced as in Rust:
  - unary `-`, `!`, `&mut` and deref `*` are recognised from the token before them;
  - turbofish `::<T>`, `|event|` handlers, `@id(...)` attributes and the `advertises: [hunger +75]` deltas are special-cased.
- **Blank lines.**
  - At most one blank line is kept, at statement and member boundaries.
  - One blank line is always put between top-level items, or object members, when either one ends in a block, or when the declaration keyword changes (`use` → `const` → `object`, `param` → `state` → `sound`).
  - No blank line follows `{` or precedes `}`.
- **Raw blocks.** A `raw { ... }` body is re-indented to the block, keeping relative indentation. One-line bodies stay `raw { ... }`.
- **No width-based reflow.** Line breaks inside groups stay under the author's control. This keeps the rules local and idempotent.

`sims-antics fmt [--check] [--stdout] <path...>` formats files in place. It walks directories for `*.antics` and skips symlinks. `--check` lists unformatted files and exits 1. `--stdout` prints one file.

## Decision 2: the decompiler validates what it writes

`decompile_iff_to_v2(iff, options)` writes one `object`, from the file's first OBJD. A later OBJD that differs from the first only in the words a v2 `variant` sets (GUID, price, catalog strings, init, main) becomes a `variant "Name" { ... }` member. Any other OBJDs (multi-tile parts) are counted, not written. The object contains:

- metadata: `guid`, `price`, and `attributes:` (the OBJD's attribute count, so `state` declarations never grow it); v2 supplies its own placeholder sprite;
- named `bcon` tables, from the file's private BCONs, with entries `e0`, `e1`, ... BCONs are chosen as the VM loads them: the last chunk of an ID wins, and only the first 128 entries (all an operand can address) are written. Table names are kept distinct ignoring case and underscores (the compiler's aliases), never take `library_consts`, and fit a 63-byte chunk label with any `{id}_{n}` suffix;
- `sound sound_N = preset(.., wav: "event", id: N);` for every FWAV v2 can declare, keeping its ID and event name (not ID 0, a shared event name, or a name longer than a chunk label);
- `extern bcon library_consts` entries (`g256_3`, `s8192_1`) for the global/semi-global constants it uses; a constant operand that addresses no table stays `raw`;
- `state attr_N @ N: i16;` for every object attribute below the attribute count that it touches, with no initializer, so `init` never writes it (higher attributes stay `raw`);
- `extern mod globals` / `extern mod semi_globals` for called library trees;
- `const GUID_XXXXXXXX` items for GUIDs used more than once;
- the interaction table as a comment (the trees it names are kept by ID; TTAB itself is not rebuilt);
- one `@id(N) @frame(args, locals, format, revision) tree` for every private BHAV. Sim-context trees (`(sim: Sim)`) are told apart from object-context trees by the `my.`/`stack.` scopes their bodies use. The OBJD's init and main trees become `init { }` / `main { }` blocks.

Each body is lifted with the existing structured `.air` lifter, parsed with the `.air` parser, and translated statement by statement into v2 idioms:

| `.air` shape | v2 |
| --- | --- |
| `while true { }` | `loop { }` |
| `while !animate(...) { }` | `animate(...);` / `animate(...) \|event\| { }` |
| `local[v] := 0; while set_to_next(1 \| 0, dest: local[v]) { } stack_obj := local[a];`, or the bare loop when v2's optimizer has dropped a restore nothing reads | `for p in lot.sims()` / `for o in lot.objects()` |
| `test_object_type(G, stack_obj)` | `self.is_type(G)` |
| `alter_budget(.., flags: 1 \| 0)` | `can_afford(..)` / `debit_funds(..)` / `credit_funds(..)` |
| `action_string`, `random`, `remove_object` | `action_label`, `random_into`, `destroy_self` |
| `stop_all_sounds`, `refresh`, `go_to_relative`, `push_interaction`, `run_tree_by_name`, `manage_inventory`, `show_string`, `set_headline`, `special_effect`, `breakpoint`, `find_best_interaction`, `grab`, `drop`, `gosub_found_action`, `notify_out_of_idle` | the PR D builtins of the same name |
| operands | `self.attr_N`, `self.data[i]`, `sim.motive[i]`, `sim.person[i]`, `Table.eN`, `library_consts.gN_M`, `temp[i]`, `event` |

Anything else becomes a minimal `raw { ... }` statement holding the original `.air`. v2 hoists the `stack_obj` save of a tree whose loops clobber it to the tree's first statement. So when that first statement saves `stack_obj` into a local that only `for`-shaped loop restores read, it is left out and v2 writes it again. The `.air` text is searched for the local too, so a use the statement walker misses keeps the save.

### Validation

The decompiler checks its own output:

1. It compiles the generated object with the real v2 compiler.
2. It compares every original private BHAV with the recompiled one: byte equality first, then `check_bhav_equivalent_combined`.

Byte equality only proves anything when the tree runs in the same surroundings. Both shortcuts (byte equality, and the unreachable tail below) therefore also need:

- the same tuning constants, compared as the VM reads them (entries past 127 have no address, and an empty table reads like a missing one);
- the same attribute count;
- the same arity, and argument-free status, for every callee the tree names.

Otherwise the checker decides, with both contexts. The checker does not model sounds. A tree whose Play Sound Event names an FWAV that the recompiled object does not reproduce is reported inconclusive, with the reason.

Each tree starts at the most readable level and moves down until it compiles and is proven:

1. **Idiomatic.** Statements are translated; single statements can be forced to `raw`.
2. **Idiomatic, object context.** Only for a Sim-context tree; it drops `(sim: Sim)`, whose `__self_obj` save adds a node.
3. **Raw.** The whole structured body in one `raw` block. Only when the structured lift succeeded.
4. **Raw, labeled.** The labeled-CFG lift, which keeps node order exactly.

A v2 compile error is mapped back by line, to the innermost statement (which goes `raw`) or else to its tree. An `.air` error is mapped to its tree. After a successful compile, every tree that is not proven moves down one level. The loop repeats until nothing changes. The finished text always goes through `format_v2_source`.

Special cases:

- **Unreachable tail.** v2 treats `raw` as opaque and appends `return true;`. After a labeled raw body that adds one trailing node no edge reaches. A recompiled tree that equals the original plus such a node counts as Equivalent, under these conditions:
  - same header, and not an indexed tree (kind 4);
  - every original edge stays in range (an out-of-range edge faults with the node count);
  - no call's inline arguments could run on into the appended node: the VM copies `2 × argc` bytes from the operands onward, across later nodes, so every call with inline arguments needs a known arity whose bytes fit before the original's end.
- **Lifecycle trees.** An init or main tree whose header v2's `init`/`main` block cannot reproduce (format other than 0x8002, or a tree still unproven at the lowest level) is written as a plain `@id(N) @frame(..) tree` instead. v2 then emits its own init/main tree for the OBJD, so the decompiled object's OBJD entry points no longer name the original IDs.
- **Catalog variants.** A variant names its init/main tree only when that tree is written as a plain `tree`; it inherits the object's own when the IDs match and the object writes that tree as its block. Trees a variant runs are written in object context, as v2 requires. A variant that cannot be spelled is left out with a comment, and so are all of them if the compiler rejects them. The report lists a variant whose GUID, price, or init/main IDs the recompiled object does not reproduce as `OBJD N (variant "Name")`.
- **Calls.** A call is written as a v2 call only when caller and callee share a context. Otherwise it stays `raw`. A tree is only as proven as the private trees it calls: a proven caller of a tree that is not proven (inconclusive, omitted, different), or of an ID the original lacks but the recompiled object has (such as v2's own `init`), is reported inconclusive. This repeats until callers of callers settle.
- **Omitted trees.** Some trees have no v2 spelling: a BHAV format outside 0x8000–0x8002, or a 0x8000/0x8001 header with more than four arguments. So does a tree that still does not compile at its lowest level, such as a full 251-node tree, which has no room for v2's appended return. Such a tree is left out with a comment, and the report gives the compiler's reason; the rest of the file is still written.
- **Resources.** The report lists behaviour-relevant chunks (`STR#`, `CTSS`, `TTAB`, `TTAs`, `SLOT`, `FCNS`, `GLOB`, `TPRP`) that the recompiled object does not carry byte for byte. The CLI prints them as a note. Tree checks cover code, constants, callee shapes and sounds, not these.

**Why validate inside the decompiler.** Lowering adds its own structure: the `__self_obj` save, local allocation, and its own loop forms. Predicting all of that in the decompiler would copy Track 8's lowering. Asking the compiler and the verifier gives the same guarantee and stays correct as lowering changes.

**Why most real trees end up raw.** Structured re-encoding renumbers nodes. The checker cannot prove a renumbered tree that yields, because a yield leaves the node index on the stack. Content written by v2 itself round-trips idiomatically. Content from other tools mostly lands on the labeled level, which is still v2 that compiles and is proven, but with `.air` inside it.

`sims-antics lift --v2 <file.iff|archive.far> [--entry <name>] [--install <dir>] [--no-catalog]` prints the decompiled object to stdout and a per-tree summary to stderr. It exits 1 if any tree has a counterexample. Callee arities come from the install catalog when the `install-catalog` feature is built: from `--install <dir>`, or else from the install found above the input path. The default `.air` lift is unchanged.

## Consequences

- The formatter is safe on any parseable file: the worst outcome is an error, never a semantic change. A token-equality failure is a bug in the formatter.
- The decompiler's readability can grow one idiom at a time. A wrong idiom cannot ship silently, because the validation loop drops that tree to a lower level.
- **Not whole-file equivalent.** The recompiled IFF has the v2 compiler's own chunk layout (sprites, OBJD words, CTSS, no TTAB). Equivalence is therefore checked tree by tree on BHAV IDs, not with whole-file `verify-equiv`, which needs identical non-BHAV chunks.
- **Known v2 gaps** (Track 8 follow-ups), reported as inconclusive rather than hidden:
  - an FWAV with ID 0;
  - two FWAVs naming the same event;
  - a tree's kind byte, which `@frame` cannot set.
- **What would reopen this:**
  - v2 gains a way to pin non-BHAV chunks (TTAB, OBJD entry points); whole-file `verify-equiv` would then apply;
  - the checker learns to prove renumbered yielding trees; real content would then stay idiomatic;
  - an AST pretty-printer becomes necessary for width-based reflow.
