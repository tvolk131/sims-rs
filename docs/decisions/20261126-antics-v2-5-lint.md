# `.antics` v2.5 Step 2A: the lint after `keep_layout` and extern frame widths

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`; new `tests/integration/v2_5_lint.rs` (12).
  No change to lowering, the parser, `v2/decompile.rs`, `open-pack/` or `docs/roadmap.md`.
- **Track:** Track 7, `.antics` v2.5 Step 2A
- **Builds on:** [`20261126-antics-v2-5-compiler.md`](20261126-antics-v2-5-compiler.md) (#503:
  `#[bhav(keep_layout)]`, `#[frame(args = N | unknown)]` on extern fns, option bindings read
  in place in fixed frames, `get_up`)

Every source in the tests is synthetic. The corpus checks below ran on copies of Track 9's
`lift-v2-4-final-{beh,strict}` (1,191 files each) with `ensure_standing` renamed to `get_up`,
as #503 did. They are not committed.

## Decision 1: `duplicate_std_global` compares frame widths

`same_abi` now also requires the same `ExternFnDecl::frame_args()`. That is the declared
`#[frame(args = N)]`, the positional count when there is none, and `None` for `unknown`.

A local declaration of a `std::globals` ID with `#[frame(args = 6)]` or `#[frame(args =
unknown)]` vetoes the tree's narrow frame for the whole object, so a literal call to it keeps
the tree's layout (#503 Decision 2). Dropping the declaration for the std one, as the finding
said, would change the bytes. Such a declaration is no longer a duplicate. Neither is a narrow
width other than the std one (`args = 2` on a one-parameter fn). A width equal to the std
one (`args = 1` there, or none) still is, and still builds the same bytes when dropped.

## Decision 2: `manual_raw_has_builtin` says when the native form needs `keep_layout`

A raw block that keeps its layout (nearly every block this rule finds: `scope(..)`, `pad` and
most primitives do) keeps the backend pass off the whole tree. The native statement does not,
so the rewrite may build other bytes, or fail to build when the tree's `node_order` counts the
kept nodes. All 25 strict findings are like that. #503's `#[bhav(keep_layout)]` keeps the
pass off as such a block does when it ends the body.

The finding is unchanged: same place, same message, emitted without a build. What is new is a
trial, when the block can be written out exactly.

- **When.** Every line of the block has an exact v2 statement:
  - `remove_object(me | stack_obj[, flags: N])`;
  - `play_sound(N, false | stack_obj, padI: 0xNN, ..)`;
  - `clear_motive_changes(padI: 0xNN, ..)`;
  - the `scope(..)` reads into `temp[K]` and stores from a literal, `temp[J]` or `param[J]`,
    each with its own index in place of the help's `temp[..]`.

  The tree must also be a `#[bhav] fn` with no `keep_layout` yet, in a file that builds, and
  the block must hold no comment. Forms the help spells with `..` get no trial
  (`find_best_object` as a test, `set_to_next`, `reach`, the `RAW_BUILTINS` table). Neither
  does a lifecycle or interaction body, which has no `#[bhav(..)]`.
- **What.** The block is rewritten to those statements and built.
  - If that builds the file's bytes, nothing changes.
  - If it does not, it is built again with `keep_layout` added to the tree's `#[bhav(..)]`. The
    edit goes before its `)`, after a `(` or a trailing comma without one, or as
    `#[bhav(keep_layout)]` for a bare `#[bhav]`. Other attributes and `pub` may stand between
    it and `fn`.
  - If only the pinned form builds the same bytes, the help adds: "and add `keep_layout` to
    the tree's `#[bhav(..)]`: this block keeps the tree's layout, and the native form builds
    the same bytes only with the pin".
- **Limits.**
  - Each block is tried on its own, as the finding stands on its own. Two anchors of one tree
    are not built together.
  - A file takes at most 8 trials (`LAYOUT_BUDGET`, two builds each), in source order.
  - The pin is never offered when the plain rewrite already builds the same bytes
    (`a_native_form_that_builds_alike_needs_no_pin`).
- **Corpus.** Every finding has a trial. 25 of 25 strict findings and 2 of 2 behavioral ones
  get the pin advice.
  - Applied by hand:
    - all 27 build byte-identical with the pin;
    - without it, 24 strict ones fail to build (`node_order` counts more nodes than the tree
      compiles to), and the other strict one and both behavioral ones build other bytes.
  - No other finding changed in either corpus, and the lint printed nothing to stderr.

The byte check of the pending findings (`verified`) and the new trial share one builder
(`same_bytes`), which builds a pack file against its pack as before.

**One snapshot of the file.** Before, the baseline of a pack file (`expand_for_lint`) was
built from the file read again from disk, while every trial builds edits of `input.source`,
the text the linter read. A file that changed in between was checked against the wrong
bytes: a block whose native form builds alike could get the pin advice. This gap predates
this step and applied to every byte-checked finding. The baseline is now built from
`input.source` too, under the same label and pack config, with `read_v2_checked_file`'s path
checks kept (`a_pack_file_is_checked_against_the_text_the_linter_read`). Other files of the
pack that the file imports are still read from disk on each build. The path checks also still
read the file: a file that stops decoding mid-lint loses its byte-checked findings
(`frame_headroom`, the pin advice). That fails closed, so no advice is ever wrong.

## Decision 3: `prefer_if_let` in fixed frames needs no lint change; tests pin it

#503 reads an option binding in place where a fixed `locals = N` frame cannot hold its copy,
so the rewrite that `prefer_if_let` trial-builds now builds there. The lint is unchanged.

- **Corpus.** `prefer_if_let` now reports 68 behavioral findings (was 6) in 46 files, and 36
  strict ones (was 0) in 30 files. Every one is in a tree whose header fixes `locals`.
  - Of these, 65 and 33 are `let .. else`; the rest are `if let`.
  - Applied together per file, each file builds byte-identical, passes `fmt --check`, and
    has no `prefer_if_let` left.
- **Tests.**
  - A `locals = 0` tree takes `let .. else` and `if let`, and so does a `locals = 1` tree
    with a local of its own.
  - The help renames only the reads before a call that may write the temp.
  - None is reported where the copy must stay: a `keep_layout` or indexed (`kind = 4`) tree,
    or the tree's own `$Local:N` text. There, the trial build of the rewrite fails.

## Decision 4: header-field audit

No table in the lint lists `#[bhav(..)]` header fields. It reads headers through the parser,
whose error text (`v2/attrs.rs`) and the `.air` parser (`src/parser.rs`) already list
`keep_layout`, as does the `TreeMemberDecl::frame` rustdoc (#503). `fmt` is token-based and
keeps the bare flag. Nothing changes.

## Follow-ups

- **Path checks without reading the file.** Split `read_v2_checked_file` (`v2/loader.rs`) so
  that the lint can check a pack file's path and load its config without decoding the file
  again.

- **Track 9 (lift).** Write the 25 strict anchors as their native statement plus
  `keep_layout`. The pin advice now names exactly those, and `prefer_if_let`'s 68 + 36 apply
  byte-identically.
- **A joint trial per tree** (two anchors and one pin together), if a tree with more than one
  ever shows up. The corpora have none.
- **Exact text for the remaining forms** (`set_to_next`, `reach`, `find_best_object` as a
  test), so their blocks can be tried too.
- **The pin for lifecycle trees** (`#[frame(keep_layout)]` on `main` / `init`), which take no
  `#[bhav(..)]`.
