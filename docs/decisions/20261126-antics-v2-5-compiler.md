# `.antics` v2.5 Step 1A: `keep_layout`, extern frame widths, fixed-frame option bindings, `get_up`

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/{ast,parser}.rs` (the `.air` `keep_layout` header field)
  - `src/v2/{ast,attrs,parser,loader}.rs`, `src/v2/lower/{mod,tree,trees,env,callable}.rs`,
    `src/v2/mir/{pipeline,frame}.rs`, `src/v2/std/globals.antics`
  - new `tests/integration/v2_5_compiler.rs`; `tests/integration/v2_typed_std.rs` (382)
  - no edits to `src/v2/lint.rs`, `src/v2/decompile.rs`, `open-pack/` or `docs/roadmap.md`
- **Track:** Track 8 (compiler), `.antics` v2.5 Step 1A
- **Builds on:**
  - [`20261126-antics-v2-4-lift-dead-false-and-v24-features.md`](20261126-antics-v2-4-lift-dead-false-and-v24-features.md)
    (#499: the 25 strict layout anchors, the 15 `semi_globals_wide` trees);
  - [`20261126-antics-v2-4-dead-false-error.md`](20261126-antics-v2-4-dead-false-error.md)
    (#498: wide-inline-call checks, the object-wide `$Local:N` text guard);
  - [`20261126-antics-v2-4-step2-light-mode-lint-and-lit-lamp.md`](20261126-antics-v2-4-step2-light-mode-lint-and-lit-lamp.md)
    (#497: the 62 option-binding proposals that missed, the `use .. as self` gap);
  - [`20261125-antics-v2-typed-std.md`](20261125-antics-v2-typed-std.md) (#468: 382 stayed
    `-> Result`).

Every source, library name and tree name in the tests is synthetic. The corpus probes below
were run once against the Track 9 lifts and are not committed.

**No regression on the lifts.** Both final lifts were expanded with `origin/main`'s compiler
and with this one. That is all 2,382 files of `lift-v2-4-final-strict` and
`lift-v2-4-final-beh`, with `ensure_standing` renamed to `get_up` in the new run (Decision 5).
Every file gives the same `.air`. open-pack builds byte-identical.

## Decision 1: `#[bhav(keep_layout)]` pins the layout without a raw anchor

`#[bhav(.., keep_layout)]` (and `#[frame(keep_layout)]` on a lifecycle tree) keeps the
backend pass off the tree. Its nodes and frame words stay as lowered, exactly as for a tree
whose body ends in a `raw` block that keeps its layout. The `.air` header gets
`keep_layout: true`; the `.air` compiler accepts the field, but compiles no differently with it.

- **Where it applies.** `finish` sets the tree's `keep_layout` from the pin, the way a raw
  anchor at the end of the body would. Lowering itself is unchanged: an inline branch lowered
  before a raw block sees no kept layout either. So the pin is the anchor's exact equivalent,
  not a stronger "keep everything from the first statement".
- **Opt-in.** No source changes bytes unless it writes the flag. `fmt` keeps it as a bare
  flag (`#[bhav(id = 4200, keep_layout)]`). A value (`keep_layout = 1`) or a second copy is
  refused, as for `share_tails`.
- **Corpus.** All 25 strict `manual_raw_has_builtin` anchors, rewritten to their native
  statement with `keep_layout` added, expand to the same `.air` in 22 trees. The other 3
  differ only in spelling `remove_object(0, ..)` as `remove_object(me, ..)`, which assemble to
  the same instruction (`keep_layout::pins_the_layout_a_raw_anchor_kept` compares the nodes).

## Decision 2: an extern fn may declare its callee's frame width

A literal (inline) call to a tree whose frame holds more than four arguments reads the rest
from the bytes of the following nodes. So `share_tails`, `node_order` and `dead_false` list only
proven-narrow callees (`narrow_callees`). An extern fn's positional parameters are trusted to be
the callee's whole frame. A tree called through the temps declares none, so the lift could not
put it in its library block (`extern "Lib" mod`) without claiming a frame of 0. That is why the
15 trees went in an untagged `semi_globals_wide` block.

An extern fn now takes `#[frame(args = N)]` or `#[frame(args = unknown)]`:

- `args = N` is the frame width, and must hold the positional parameters (refused otherwise).
  `#[temp(N)]` parameters go in the temps the call passes, and are not checked against it.
- `args = unknown` proves nothing: no literal call to the tree is taken as narrow, and a
  source literal call keeps the layout like any wide call.
- `frame_is_narrow` and the extern's call signature both read the declared width. A tree stays
  narrow only when every declaration the object can see gives a width of 4 or less, as before.
  Fail-closed:
  - a wide or unknown declaration vetoes a narrow one;
  - a semi-global still needs its linked library's block.
- **A literal call not proven narrow keeps the layout.** Before, only a call whose own
  declaration was wide kept it. A call through a narrow declaration that another declaration
  vetoes (or a semi-global with no library block) was refused under `share_tails`,
  `node_order` and `dead_false`, but otherwise the backend pass could still rewrite the nodes
  after it, which the callee may read. It now keeps the layout too. As for any wide call, that
  also stops inline-branch lowering for the rest of the tree (`try_lower_inline_branch` checks
  the kept layout), unlike the `keep_layout` pin (Decision 1). The gap predates this
  step; no lifted file or open-pack tree has such a call (see the sweep above).
- **Discarded code keeps nothing.** A wide call inside discarded code (`if false { .. }`)
  emits no call, so it no longer pins the tree's layout.
- Other header fields (`locals`, `kind`, ...) describe a tree's own body and are refused on an
  extern fn, as is a second `#[frame]`.
- Two declarations of one name that merge must agree on the width, as declared or implied by
  the positional count (`#[frame(args = 0)]` on an argless fn merges with the bare fn), and
  `unknown` only with `unknown`. `extern_fn_text` (the merge diagnostics) prints it.
- **Corpus.** Each of the 10 files with a `semi_globals_wide` block was rewritten: the block's
  fns (ids 32767 ×8 and 65535 ×7) moved into the file's `extern "Lib" mod semi_globals` with
  `#[frame(args = unknown)]`, and the call sites became `semi_globals::`. All 10 expand to the
  same `.air` as the lifted file. `unknown` is the honest spelling for them: neither id is in
  the catalog, and every call passes the temps.

## Decision 3: option bindings read in place where a fixed frame cannot hold their copies

#495 reads an option binding (`let Some(v) = f() else { .. };`, `if let Some(v) = f() { .. }`)
in place from the temp the call filled, but only where dropping the copy is unobservable. Off,
among others, in an object whose text may name frame words (`$Local:N`, #498's object-wide
guard).

A probe applied the 62 behavioral proposals that #497 found still missing. Nearly all were
trees with a fixed header (`args = 0, locals = 0`), in objects behind that guard. There, the
copying form does not build at all: "declares 0 locals but requires at least 1". The
`me`-with-no-header cases #497 named are those same trees with the header dropped.

- **The rule.** A tree whose header fixes its frame (`locals = N`), and whose finished body
  needs more words than `N`, is finished again with every option binding offered for reading
  in place, although other trees' text may name frame words. The result is kept only if it
  then fits in `N` words. So a source that builds today keeps its bytes. Only a source the
  `.air` compiler would refuse changes.
- **Its own text declines it.** If the tree's own text may name a frame word, a copy may fill
  the word that text shows, so the rule does not run and the `.air` compiler's error stands
  (`the_trees_own_frame_word_text_keeps_the_copy`). The copies are kept or dropped all
  together: the rule tries no partial set.
- **Why it is sound.** Two object-wide guards keep the copies: frame-word text in any tree (a
  later frame's `$Local:N` text may show a word this frame left behind), and a kept layout or
  layout staging elsewhere in the object (another tree's call may read a word left there). The
  retry skips both, because they only stop the optimizer from choosing bytes that could be
  told apart from the copying form. Here no copying form builds, so no existing bytes or
  frame residue can differ. The
  header pins the frame, and the in-place reads see exactly the value the copy would have held:
  `alias_option_temps` still proves that nothing writes the temp between the call and each read.
- **Still excluded:**
  - a kept layout (`keep_layout` or a raw anchor);
  - a wide inline call, which reads the nodes after it;
  - main's rules, since the copy is an Expression whose scratch-pointer write global 34 reads;
  - an indexed (`kind`) tree entered at a numbered node;
  - a binding inside a `#[temp]` loop.
- **The word count** (`mir::frame_words`) is `.air`'s own scan (`body_local_usage`):
  - the named `let` / `local` names after the explicit locals and the `min_locals` floor;
  - the `idle` scratch word, when the tree uses both `idle` and `param[0]`
    (`the_idle_scratch_word_counts`).

  A body `.air` cannot parse, or with raw `op` bytes, gets no count, and the rule stays off.
  A count above `N` means the `.air` compiler would refuse the tree, so the rule never turns
  on for a source that builds.
- **Frame-word text is still respected.** A frame with room for the copy keeps it beside such
  text (`a_frame_with_room_keeps_the_copy`). Without such text, nothing changes.

A probe rewrote every `if !take_elapsed_periods(..) { .. } <read of temp[0]>` and
`find_companion` shape in the final lifts to `let .. else` / `if let`, and compared bytes:

| Corpus | Shapes | Same bytes, before | Same bytes, now |
|---|---:|---:|---:|
| `lift-v2-4-final-beh` | 106 | 6 | 43 |
| `lift-v2-4-final-strict` | 91 | 0 | 19 |

The rest are out of reach on purpose:

- **Raw blocks.** A raw block keeps the layout: 46 behavioral and 58 strict trees, most of them
  a raw `local[0] := ..` before the binding.
- **The guard.** A frame of one word that has room for the copy, beside `$Local:N` text, keeps
  the copy: 14 + 3 behavioral and 10 + 4 strict trees. The +4 strict trees then fail their
  `node_order` count. These include the `locals = 1` maintenance-NPC and hot-tub trees, whose
  original leaves the word unwritten.

## Decision 4: `use .. as self;` is refused

#495 refuses `self` as every kind of binding, but an import alias could still be named `self`.
The constant would then answer to `self` in value position, while `self.field` stayed the
running object. The parser now refuses an import named `self`, aliased or not, in both
`use a::b as self;` and `use a::{b as self};`: "'self' always names the running object and
cannot be imported as a name; choose another alias".

## Decision 5: `std::globals`: 382 is `get_up() -> bool`; 282 and 333 keep their typed answers

- **382** was `ensure_standing() -> Result` (#468). Its false exit is an answer callers branch
  on: the Sim has no container, or the way up fails. It is now `#[id(382)] fn get_up() -> bool;`
  with a clean-room comment. A local `-> Result` declaration of 382 beside
  `use std::globals::*;` now calls the tree another way, and the merge refuses it ("calls tree
  382 two ways").
  - The lift picks the name up from `std::globals`.
  - Already-lifted files that call `globals::ensure_standing()` must be lifted again. They
    spell the call as a branch (`if !..`, `return ..`), so their bytes do not change.
- **282** (`take_elapsed_periods(period) -> Option<i16>`) and **333**
  (`find_companion() -> Option<ObjectId>`) have carried the typed bool + value ABI since #480
  (`#[ret(temp = 0)]`). `#[temp(N)]` parameters combine with an `Option` answer in any extern
  (`temp_params_with_option`).
  - 282's period stays positional: every call in the corpus passes it inline. `#[temp(0)]`
    would turn those calls into `call 282(temps)`, which are other bytes.
- **Names.** Other `std::globals` names sit close to descriptive labels. Renaming them is out
  of this step's scope and left as a follow-up. This ADR quotes none of them.

## Review

Codex GPT-6.2 Sol, round 1, reported four findings. Sol reproduced each one, and all four are
fixed, with tests:

1. **High.** A literal call through a narrow declaration that another declaration vetoes
   recorded the wide call but left the layout open to the backend pass. It now keeps it
   (Decision 2; `a_literal_call_not_proven_narrow_keeps_the_layout`).
2. **Medium.** The fixed-frame retry ran beside the tree's own frame-word text, and could drop
   a copy whose word that text shows. The retry now declines there (Decision 3). The
   pipeline's `fit_frame` bypass is gone, so its frame-word-text gate holds for every alias.
3. **Medium.** A wide call in discarded code pinned the layout. Now it does not
   (`a_discarded_wide_call_keeps_nothing`).
4. **Medium (fail-safe).** The word count missed the `idle` scratch word. It is now `.air`'s
   exact scan (Decision 3).

Round 2 went to a Claude Opus 5.5 reviewer, because Sol's run was stopped by a content
filter. It found no high or medium problems and confirmed the four fixes. Its low findings:

- the merge compared the width's spelling, not the width: now fixed (Decision 2);
- the soundness argument named only one of the two guards the retry skips: corrected
  (Decision 3);
- an unproven-narrow call also stops inline-branch lowering: now documented (Decision 2);
- the `duplicate_std_global` lint ignores the width: left to Track 7 (Consequences), since
  this step does not edit `lint.rs`.

## Consequences

- **Track 9 (lift):**
  - write `keep_layout` in place of the 25 strict raw anchors;
  - lift the 382 calls again as `get_up`;
  - put the 15 `semi_globals_wide` trees in their library block with
    `#[frame(args = unknown)]`, and a `#[frame(args = N)]` where the catalog gives an arity
    above the positional count;
  - offer `let .. else` / `if let` again in fixed-frame trees.
- **Track 7 (lint):**
  - list `keep_layout` in the header-field tables;
  - `prefer_if_let`'s trial builds now keep the fixed-frame proposals;
  - `manual_raw_has_builtin` can propose `keep_layout` for an anchor;
  - `duplicate_std_global` should compare the frame width (`ExternFnDecl::frame_args`) in
    `same_abi`: a local `#[frame(args = 6)]` or `unknown` declaration of a `std::globals` id
    keeps the layout, so dropping it for the std name would change bytes.
- **Follow-ups:**
  - raw `local[N]` blocks that keep the layout only because of an explicit local;
  - a `keep_layout` that also covers lowering (inline branches);
  - clean-room names for the rest of `std::globals`.
