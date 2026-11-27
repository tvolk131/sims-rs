# `lift --v2` v2.6 step 2B: dead-node pins, `keep_layout` before the idioms go, `local[N]` writes

*Decided 2026-11-27.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/v2/decompile.rs`
  - `crates/sims-antics/src/v2/std/globals.antics` (the 4 deferred renames only)
  - new `tests/integration/v2_6_lift_finale.rs`; `tests/integration/v2_5_lift_locals.rs`
    (fixtures the new step now lifts native)
  - the renames' call sites in `tests/integration/{v2_lift_wave3,v2_lint_wave3,v2_lint_wave6,
    v2_panic_and_discard,v2_pr_i4_and_std_utils,v2_rust_syntax}.rs`
  - no edits to `src/v2/lint.rs`, `open-pack/` or `docs/roadmap.md`
- **Track:** Track 9 (`lift --v2`), `.antics` v2.6 step 2B
- **Builds on:**
  - [`20261127-antics-v2-6-compiler.md`](20261127-antics-v2-6-compiler.md) (#512, step 1:
    writable `local[N]`, `dead_nodes { }`, `match_tree_pin_quarantined`);
  - [`20261127-antics-v2-5-lift-finale.md`](20261127-antics-v2-5-lift-finale.md) (#509: the
    baseline `lift-v2-5-final-*`, the strict raw census);
  - #515 (step 2A, Track 7): lint advice for a raw `local[N]` block that builds natively.

As before, nothing changes what is claimed about a tree. Every tree is rebuilt and checked,
strict or `--behavioral`, and a tree that a change breaks falls back.

## Context

#509 left three gaps for Track 8, and #512 closed them in the compiler:

- 3,836 of the 4,961 strict raw-fallback trees keep nodes no branch reaches. A structured
  body cannot produce them, so #512 added a `dead_nodes { }` footer and
  `match_tree_pin_quarantined`, which predicts a pin with those nodes split off.
- The raw local writes (3,722 strict, 2,762 behavioral by #509's census) wait for a
  writable `local[N]`. #512 made it a place; like a read, a write keeps the tree's layout.
- Two strict trees (two alarm objects' tree 4098) need identical `if` arms kept unmerged.
  `#[bhav(.., keep_layout)]` does that, but the lift only reached it for a last raw anchor.

## Decision 1: pins carry the original's unreachable nodes

`pin_nodes` predicts its pin with `match_tree_pin_quarantined`. The plan's pin becomes a
`QuarantinedPin`: the header fields (`dead_false = error`, `share_tails`, `node_order`), when
any, and the dead nodes.

- The header is printed as before. A non-empty `dead_nodes` list is printed as
  `node_order::dead_nodes_block` just before the tree's closing `}`, one level in.
- `share_large`'s pin is wrapped as a `QuarantinedPin` with no dead nodes.
- An indexed tree (kind 4) is never pinned, as before: any node may be its entry.

A pin is still a prediction from the bytes: a pinned tree that is not proven loses it. A
tree with nothing but dead nodes to pin is pinned with the footer alone.

## Decision 2: `keep_layout` is tried before an idiom is dropped

An unproven tree used to give up its idioms first (extras, wave 3, wave 2, names, ...). Some
trees fail only because the backend pass rewrites them: it merges two identical `if` arms,
or forwards a store to a named local. Dropping idioms until a raw block kept the layout
proved them, but left raw code.

The fallback chain now tries `keep_layout` (`Plan::keep_layout`) right after `pin_nodes`:

- Only for a tree at an idiomatic level written with a `#[bhav(..)]` header: a private
  tree, or a lifecycle block written whole as one (standalone, or framed with its header;
  Sol review round 2). A lifecycle block written `#[id(..)]` has none.
- Once per tree and level (`layout_tried`). A tree not proven with it drops it
  (`layout_kept` → off) and the chain's later steps run in the same round, so the step costs
  no round when it fails.
- `drop_layout` runs right after `drop_pin` in both compile-failure chains, so a header the
  build refuses loses it first.
- A pin is worked out again when the tree's text is written with `keep_layout`: the build
  it is predicted from then keeps what the backend pass merged. A tree that needs both the
  layout and a pin (identical arms and a dead node, say) gets both (Sol review round 1).
  `share_large`'s `share_tails` pin is keyed the same way (round 2).
- The pin is printed only on the text it was planned for (or on a tree the locals pass
  rewrites), so a text that changed under it is tried again plain.

The two alarm trees now lift with a native `if`/`else` and `keep_layout`, and are proven.
So do the synthetic fixtures of #505 whose stores and reads were raw only because naming the
locals let the optimizer forward them (`v2_5_lift_locals` now asserts that).

## Decision 3: locals left by number are written as `local[N]` too

`read_locals_in_place` (#505) runs once the lift has converged. It now writes stores as well
as reads: `=`, `+=`, `-=`, `dec_gt(&mut local[N], ..)` and the reads that share their raw
blocks. The translator's `place` serves both (`lvalue` is a write, `rvalue` a read); a word
is `local[N]` only through `local_word`.

The pass has two stages, each with its own `LOCAL_READ_BUILDS` (4) whole-object builds:

1. **Reads only.** A tree whose nodes come out different keeps its reads raw
   (`raw_local_reads`).
2. **Writes too** (`Plan::local_writes`), on stage 1's blame. A blamed tree that wrote a local
   keeps its writes raw (`raw_local_writes`) and its reads get another try. When stage 2 does
   not build the same bytes within its builds, or writes in place fewer trees than stage 1,
   stage 1's result stands.

So writes never cost a tree its reads. A first version blamed reads and writes in one loop.
In 12 strict files the budget then ran out before it converged, and those trees lost reads
that `lift-v2-6-c` had.

Every candidate is checked against the whole object's bytes, so no verdict changes.

The corpus probes found three spellings that blamed a tree in stage 1 although its reads were
fine. Each is fixed in the translator:

- **`distance_to(.., from: self.id)` in an inherit tree.** An inherit tree refuses `self`;
  the runner's id is `me.id`, as the translator already spells it everywhere else.
  `distance_handle` now gives `me.id` (and nothing but `stack_obj` otherwise) there. The main
  loop gains from this too: such a tree no longer drops its inherit context or its idioms.
- **`local[N].is_type(..)`.** `local[N]` is a plain word, not a handle, so a type test of an
  object a local holds stays raw.
- **A word past the declared frame.** v2 sizes a frame to cover every `local[N]`, so a
  `local[5]` in a 4-word frame does not build. `local_word` now refuses any word at or past
  the tree's declared locals.

`set_to_next(.., dest: local[N])` goes through the same `lvalue`, but only in a tree written
with its extra idioms (as for `dest: stack_obj`). Of the 107 such builtins the locals pass
left raw strict (82 behavioral), all in trees written without them, `polish` writes 5 (1)
natively (below); 101 (81) stay raw.

The late lint-spelling pass can rewrite a typed flag test that `polish` had already turned
into a method. When it changes the text, the flag-method groups (`flag_methods`, extracted
from `polish`) run again, so such a tree does not come back with a `prefer_method_syntax`
warning.

### What the pass leaves raw, block by block

Since #515, lint `manual_raw_has_builtin` reports a `raw { }` block of `local[N]` stores
(`DST OP SRC` lines) whose native form builds the same bytes. The locals pass blames whole
trees, so a tree with one store that builds other bytes kept all its stores raw, and the
v2.6 sweeps drew 181 such findings strict and 67 behavioral.

`polish`'s raw-builtin step (`raw_builtin_spellings`) now spells those lines too
(`raw_local_line`, the lint's forms). Each block is tried on its own, and with
`keep_layout` when refused alone, like the step's other builtins. The block's statements
leave the tree's raw count.

The lint also spells `set_to_next(.., dest: local[N])` with its literal mode and GUID. The
step writes it as the lifter writes the step itself: the mode by name and the GUID by its
`GUID_XXXXXXXX` constant, since a GUID-sized literal draws `magic_guid_literal`. The
generator clears a constant's declaration that only raw text needs, so it now keeps one
that such a line would name, and the step clears the ones still unused after it. A GUID
with no constant leaves its block raw.

## Decision 4: the deferred `std::globals` names

#512 renamed nineteen `std::globals` trees to clean-room descriptions, and deferred four whose
names `decompile.rs`'s tests spell:

| id | old | new |
|---:|---|---|
| 271 | `init_object` | `set_up_object` |
| 272 | `standard_entry` | `claim_stack_object` |
| 273 | `standard_exit(delay)` | `release_stack_object(delay)` |
| 363 | `am_i_a_child` | `sim_is_child` |

Ids, signatures and parameter names are unchanged, so every call builds the same bytes.
open-pack calls none of them and builds byte-identical to `origin/main`. Lifted files that
call an old name must be lifted again.

## Results

Full sweeps of the private corpus into `cache/track9/lift-v2-6-final-{strict,beh}`
(gitignored, never committed), against `lift-v2-4-final-*` and `lift-v2-5-final-*`. Both
modes: 1,191 objects lifted, 0 errors or timeouts, 26,206 trees, 0 DIFFERENT, 0 fmt-dirty
files. Compared file by file with v2.5-final there are 0 idiomatic or proof regressions. The
files with more raw statements all have fewer raw-fallback trees: a whole tree became native
and kept a few inline raw statements.

**Strict**

| metric | v2.4-final | v2.5-final | v2.6-final |
|---|---:|---:|---:|
| idiomatic trees | 21,244 (81.1%) | 21,244 (81.1%) | **24,461 (93.3%)** |
| identical / equivalent | 4,234 / 3,310 | 4,234 / 3,310 | 4,735 / 2,809 |
| raw-fallback trees | 4,961 | 4,961 | **1,744** |
| whole raw blocks / lines | — | 5,123 / 291,835 | 1,870 / 153,065 |
| inline raw statements | 25,087 | 24,484 | 31,575 |
| inline raw lines | 100,604 | 98,795 | 112,787 |
| total lines | 966,953 | 966,640 | 967,406 |
| `dead_nodes` footers | — | — | 3,214 (783 files) |
| `keep_layout` trees | — | 25 | 161 |
| native `local[N]` writes / reads | — / — | 0 / 743 | 1,377 / 968 |
| raw local writes, inline / whole | — | 3,501 / 11,251 | 2,907 / 6,610 |
| lint warnings (its own linter) | 26 | 1 | 1 |
| lint warnings (v2.6's linter, with #515) | — | 48 | 1 |

**Behavioral**

| metric | v2.4-final | v2.5-final | v2.6-final |
|---|---:|---:|---:|
| idiomatic trees | 25,590 (97.6%) | 25,590 (97.6%) | **25,593 (97.7%)** |
| identical / equivalent | 1,143 / 6,401 | 1,143 / 6,401 | 1,143 / 6,401 |
| raw-fallback trees | 615 | 615 | **612** |
| whole raw blocks / lines | — | 680 / 57,370 | 677 / 57,168 |
| inline raw statements | 12,326 | 11,894 | **10,397** |
| inline raw lines | 57,502 | 56,116 | 53,117 |
| total lines | 912,940 | 912,537 | 910,039 |
| `dead_nodes` footers | — | — | 1 |
| `keep_layout` trees | — | 2 | 92 |
| native `local[N]` writes / reads | — / — | 0 / 591 | 797 / 608 |
| raw local writes, inline / whole | — | 2,566 / 3,118 | 1,372 / 3,111 |
| lint warnings (its own linter) | 9 | 1 | 1 |
| lint warnings (v2.6's linter, with #515) | — | 8 | 1 |

A dash means the metric did not exist in that version. #515's `manual_raw_has_builtin`
advice drew 181 findings strict and 67 behavioral from this step's sweeps before `polish`
spelled raw local blocks; it draws none now. The raw statement count goes up in
strict mode only because 3,217 trees that were whole raw blocks are now native with a few
inline raw statements left. The whole-block lines fall by 138,770. A pinned tree proven with
`keep_layout` names its locals (`#[slot(N)] let`), so the pin retry of round 1 moved some
trees from native `local[N]` to named slots (157 more slot bindings strict).

The remaining lint warnings, one per mode:

- **strict, `frame_headroom`:** a tree that v2.5 left as a whole raw block is now native, and
  it compiles to 201 nodes. The warning is an advisory about how close a tree is to the
  251-node limit. Splitting the tree would build other bytes.
- **behavioral, `collapsible_if_chain`:** the same warning v2.5-final had, in the same tree.

## Consequences

- Strict raw-fallback trees fall from 4,961 to 1,752. The dead-node census that #509 counted
  (3,836 trees) is now mostly native; the rest fail for other reasons (raw statements the
  translator cannot spell, or pins the proof refuses).
- Most remaining raw local writes are in pinned trees whose other statements stay raw
  (`my.data[11]` reads, inherit spellings). They need translator spellings, not locals work.
- **Track 9 follow-up:** `set_to_next(.., dest: local[N])` in trees written without their
  extra idioms (101 strict, 81 behavioral still raw); the translator spells the step only
  with them.
- **Track 7:** `manual_raw_has_builtin`'s advice for a raw `set_to_next(.., guid: G, dest:
  local[N])` block spells the GUID literal, which then draws `magic_guid_literal`. It could
  name the constant instead, as the lift does. A native tree near the node limit draws
  `frame_headroom` as designed.
- **Track 8:** nothing new.
- Lifted files that call the four renamed `std::globals` trees by their old names must be
  lifted again.
