# `.antics` v2.6 Step 2A: the lint after writable `local[N]` and `dead_nodes`

*Decided 2026-11-27.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/v2/lint.rs`;
  - new `tests/integration/v2_6_lint.rs` (15 tests);
  - one case of `v2_5_lint.rs` and the lint-clean helper of `v2_5_lift_locals.rs`, adjusted
    (see below).

  No change to lowering, the parser, `v2/decompile.rs`, `v2/structure_lift.rs`,
  `v2/std_globals.rs`, `open-pack/` or `docs/roadmap.md`.
- **Track:** Track 7, `.antics` v2.6 Step 2A
- **Builds on:**
  - [`20261127-antics-v2-6-compiler.md`](20261127-antics-v2-6-compiler.md) (#512: writable
    `local[N]`, `dead_nodes { .. }`, `v2::loader::check_v2_file`);
  - [`20261126-antics-v2-5-lint.md`](20261126-antics-v2-5-lint.md) (#507: the `keep_layout`
    trial).

Every source in the tests is synthetic. The corpus checks below are not committed. They ran on
Track 9's `lift-v2-5-final-{strict,beh}` and on `lift-v2-6-e-strict`, a v2.6 lift with
`dead_nodes` footers in 783 of its files. Each corpus has 1,191 files.

## Decision 1: the linter checks a pack file's path and never reads the file again

`pack_config` now calls `check_v2_file(path, Some(pack))` and takes its `config`. Before, it
called `read_v2_checked_file`. Both run the same path checks: a regular file, no symlinks, and
inside the pack root. The new call does not decode the file.

The lint builds `input.source`, the text it read once. A file that changed or stopped decoding
on disk mid-lint used to lose its byte-checked findings, because `pack_config` failed, so no
build ran. That was Sol's fail-closed finding on #507. Those findings now stand:

- the `keep_layout` advice;
- the `local[N]` findings below;
- `frame_headroom`;
- the `verified` rules.

A path that fails its checks still fails closed. Nothing is built, and only findings that need
no build stand.

- **Tests.**
  - `a_pack_file_that_stops_decoding_on_disk_keeps_its_byte_checked_findings` writes invalid
    UTF-8 to the pack file on disk and hands the linter valid text. It fails under the old
    double read, which lost the pin advice and the `local[N]` finding.
  - `a_pack_file_whose_path_fails_its_check_has_no_byte_checked_findings`: with no file at the
    path, only the unbuilt finding stands, without advice.

## Decision 2: `manual_raw_has_builtin` spells raw `local[N]` lines natively, on a build

Since #512 a v2 body reads and writes `local[N]`. A raw `.air` line that names `local[N]` now
has a v2 statement (`raw_local_form`) when it is `DST OP SRC`, single-spaced as the `.air`
printer writes it:

- **DST:** `local[0..=255]`, `temp[0..=7]`, a `param[N]` below the tree's compiled argument
  count, or `stack_obj`.
- **SRC:** one of those, or a literal from −32768 to 32767.
- **OP:** `:=`, written as v2 `=`; `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`; or
  `flag_set` / `flag_clear`. Each is spelled the same in v2.
- **`set_to_next(.., dest: local[N])`:** written as the line itself, except that `.air`'s
  `into:` is written as `dest:`, the only key v2 takes. Before, only a `temp` or `param`
  destination had a spelling, and only in the help, as `..`.

A block in which any line names `local[N]` is never emitted unchecked, so there are no false
positives:

- Every line must have an exact statement. A builtin line can be one, as in #507's forms.
- The block must hold no comment.
- The finding is the `keep_layout` trial's own (`LayoutTrial::finding`). The block is rewritten
  and built. The finding is added only when the rewrite builds the file's bytes, alone or with
  the tree's pin, which the help then names. Otherwise it is dropped.
- Blocks that name no `local[N]` are reported as before.

The help writes out the whole block, every statement in order: "write the block as
`local[0] = 5; temp[0] = local[0];`; v2 reads and writes `local[N]` natively". It does not
list the distinct spellings, as the older help does. A list would drop a repeated statement
(`local[0] := 1; local[0] += 2; local[0] := 1;`), and the block that was built is the whole
one.

**Limits.**

- **`dec_gt` / `inc_lt`.** `.air` has no statement form for them. They are comparisons, so a
  raw `local[N] dec_gt X;` does not parse. Raw `dec_gt` appears only in an `if` / `while` test
  or an edge, and such a block stays raw. The brief listed `dec_gt`; there is no raw form of it
  to spell.
- **Budget.** Local findings share #507's budget of 8 trials per file, in source order. A
  finding past the budget is not reported. An `allow` of it is not stale: `layout_pins` now
  returns its untried findings, and they count as seen.
- **Where.** The trial runs in any body that builds. A functional or an interaction takes
  no pin, so there the native form is tried alone.
- **Not tried.**
  - An inline fn, or a template member that no object of the file builds, is `unbuilt`.
    Other files may inline the fn, so no byte-checked rule proposes a rewrite there.
  - A `main` that a later `main` of the object replaces is not tried. Lowering keeps only
    the last `main`, so a rewrite of an earlier one builds the same bytes whatever it says.
  - In `load`, `reset` and `cleanup`, `param[N]` is not spelled. These trees are linted
    without their compiled frame, as before, so the argument count is unknown.
  - Both are missed findings, never false ones.
- **Pins in practice.** A native `local[N]` write keeps the tree's layout just as the raw one
  does. In every test and corpus case, the plain rewrite builds the same bytes and no local
  finding takes the pin.

**Corpus.** These are new findings. `main` had none in these corpora.

| corpus | new local findings | `.air` text unchanged when the rewrite is applied |
| --- | --- | --- |
| `lift-v2-5-final-strict` | 99 | 99 of 99 |
| `lift-v2-5-final-beh` | 34 | 34 of 34 |
| `lift-v2-6-e-strict` | 217 | 217 of 217 |

- In each corpus, replacing each block with the help's text gives `.air` text identical to
  the original's, so the bytes are identical too. This check does not rely on the lint's own
  byte check.
- No other finding changed.
- In `lift-v2-5-final-strict`, 4 files reached the budget.
- Linting `lift-v2-5-final-strict` took 15.6 s wall time, against 13.2 s on `main`.

**Tests.**

- Each form, in a tree alone and after a body the backend pass rewrites: the finding stands,
  the rewrite builds the same bytes, and the result lints clean.
- A mixed block of builtins and local lines.
- Lifecycle trees.
- A Sim's body.
- The budget and its `allow`.
- Blocks left raw: a comment, an unspelled cell, an uncovered `param`, an unspelled line.

**`v2_5_lift_locals.rs`.** The lift still writes `local[N]` stores raw. Its fixture
`raw { local[0] := 5; local[1] := stack_obj; }` builds the same bytes natively, so the lifted
output now has a finding. The test's lint-clean assertion skips exactly these findings, which
are recognised by their help. Writing them natively is a Track 9 follow-up.

## Decision 3: `dead_nodes { .. }` footers lint clean, with no change

The lint reads trees through the parser, which keeps a footer out of the body (#512). No rule
sees its `op 0x0002(..)` entries.

- **Test.** `a_dead_nodes_footer_lints_clean` lints, with every rule, a footer in two
  `#[bhav] fn` trees and in a `main`. There are no findings, and each builds.
- **Corpus.** `lift-v2-6-e-strict` has footers in 783 files. Its findings on this branch are
  `main`'s one `frame_headroom` plus the 217 local findings above. Nothing else is reported.

## Decision 4: the `keep_layout` advice covers lifecycle trees

#507's pin sites were per `#[bhav] fn`. `pin_sites` now also maps each lifecycle keyword (`init`,
`main`, `load`, `reset`, `cleanup`) to its header:

| header | where the pin goes | advice |
| --- | --- | --- |
| `#[bhav(..)]` | in it | "add `keep_layout` to the tree's `#[bhav(..)]`" |
| `#[frame(..)]`, no `#[bhav(..)]` | in it | "add `keep_layout` to the tree's `#[frame(..)]`" |
| neither | a new `#[frame(keep_layout)]` before the keyword, after any `#[id(N)]` | "add `#[frame(keep_layout)]` to the tree" |

A bare attribute takes `(keep_layout)` after its name, as before.

The blocks of one kind (several `init`s, say) build one tree under one header
(`lifecycle_pin`):

- The pin goes in the block of that kind that has a header attribute, whichever block holds
  the anchor. Only one block may carry one. A `dead_nodes` footer alone is no header.
- With no header, a new one goes before the first block.
- The advice names the header's line ("… to the tree's `#[frame(..)]` on line 12"), since
  that block need not be the one holding the anchor.

A template's header is not seen, even in the same file: the lint looks only at the object's
own members. The pinned trial then fails to build, and no advice is given. That is a missing
pin advice, as on `main`; the anchor finding itself is unchanged from #507.

The tree and both lifecycle branches of `lint_members_inner` set `layout_pin`, and
`LayoutTrial` carries the advice text into the help.

- **Tests.**
  - `a_lifecycle_tree_takes_the_pin`: all five keywords bare; with `#[id(N)]`; with
    `#[frame(locals = 2)]`, `#[frame()]`, or a trailing comma before another attribute; and
    with `#[bhav(locals = 2)]`. Each gets its advice, builds the same bytes only with the pin,
    and lints clean once pinned.
  - `the_pin_goes_in_the_header_of_the_blocks_composed_tree`: two `init` blocks, with the
    anchor in the second.
  - `a_dead_nodes_footer_is_no_header_for_the_pin`.
  - `a_replaced_main_is_not_tried`.
  - `a_pinned_lifecycle_tree_gets_no_advice`.
  - The v2.5 case that pinned "a lifecycle tree gets no advice" now uses an already-pinned
    `main`.
- **Corpus.** None of the three corpora has a lifecycle tree with an anchor that needs the pin.

## Follow-ups

- **Track 9 (lift).** Write raw `local[N]` stores and reads natively: the 99 + 34 + 217
  findings above. Then drop the filter in `v2_5_lift_locals.rs`.
- **A joint trial per file** for local findings, so files with more than 8 local blocks need
  not be linted again after their first 8 are fixed.
- **Cells not yet spelled** in local lines: `my.attr[N]`, `global[N]`, `stack.<field>`.
