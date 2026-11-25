# `.antics` v2 lint, wave 4: in step with PR I-4

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`, `sims-antics lint`
- **Track:** Track 7 (`.antics` v2.1 Wave 2, Stream 4c)
- **Builds on:** [20261125-antics-v2-lint-wave3.md](20261125-antics-v2-lint-wave3.md),
  [20261125-antics-v2-pr-i4-and-std-utils.md](20261125-antics-v2-pr-i4-and-std-utils.md)

## Context

PR I-4 (#469) added these to the language:

- **`std::math`, `std::motives` and `std::budget`.** Their `pub fn`s (`min`, `max`, `clamp`,
  `abs`, `chance_percent`, the five motive loops, `try_charge`) come in through a glob import.
- **Temp-indexed and neighbor scopes.**
  - 27: `sprite_flags[temp[k]]`;
  - 30: `sim.person[temp[k]]`;
  - 31: `stack_obj.person[temp[k]]`;
  - 24: `h.neighbor.*`;
  - 32: `h.neighbor.<person word>`.
- **Person word 81, `fame`.**
- **Primitive options.** `padN:` bytes on `play_sound` and `clear_motive_changes`, `reach(..,
  slot:)` and `make_new_character`.

The linter assumed a `std` glob import brings in no functions (#469 flagged this for Track 7). It
did not know `fame`, and `manual_raw_has_builtin` did not know the new spellings.

## Decision 1: a `std` glob brings in its module's `pub fn`s

`file_callables` and `file_sure_callables` add the `pub fn`s of `std::<module>` for each
`use std::<module>::*;`.

- The names are read once from the loader's embedded sources (`loader::STD_MODULES`,
  `std_module_source`), so they cannot drift from what the compiler imports.
- A glob of any other path still makes the file's callables unknown (`None`), as before.

No `std` helper has a builtin's name today, so this changes no finding yet. It keeps the rules
that defer to user functions (`raw_native_form`'s `free`, `prefer_method_syntax`,
`untyped_engine_constant`'s builtin arguments, and the unassigned-read walk) correct if one ever
does.

## Decision 2: `fame` is a typed person word

`fame` (person word 81, `i16`, writable) joins the Sim and handle word tables, so
`untyped_engine_field` suggests `sim.fame` / `stack_obj.fame` for `sim.person[81]` /
`stack_obj.person[81]`.

Like every `untyped_engine_field` suggestion, it stands only when the edited file builds the
same bytes (wave 3, Decision 1). The lift applies these suggestions, so lifted files now print
`fame`. No lift test pins `person[81]`, and `decompile.rs` is unchanged.

## Decision 3: `manual_raw_has_builtin` knows #469's spellings

The rule stays a hint with a help and no automatic fix. A raw block is named only when every
statement in it has a v2 spelling that lowers to that statement's own `.air`. Each spelling below
is tested both ways in `tests/integration/v2_lint_wave4.rs`: the raw block gets the hint, and the
v2 statement builds the same `.iff` bytes in a Sim tree and in an object tree.

| Raw `.air` | v2 | Conditions |
|---|---|---|
| `play_sound(N [, t]) pad(i: 0xNN, ..)` | `play_sound(N, false \| stack_obj, padI: 0xNN)` | `N` a literal FWAV id. `t` is `false`, `true`, `stack_obj` or `stack_obj: bool`; the spelling names the target explicitly, so it does not depend on the tree's context. No `flags:`. No pad of bytes 0, 1 or 4, a byte past 7, or a byte named twice. |
| `clear_motive_changes() [pad(..)]` | `clear_motive_changes([padI: 0xNN])` | No arguments; pads as above, any byte 0..=7. |
| `reach(mode: 1, moves: B [, param: P])` | `reach(mode: 1, moves: B, slot: param[P])` | `P` (`.air` defaults it to 0; written `P` or `param[P]`) below the tree's compiled argument count. |
| `reach(mode: M, moves: B [, param: 0])` | `reach(mode: M, moves: B)` | Any other mode. v2 refuses `slot:` there, so a nonzero `P` stays raw. |
| `scope(12, m)` | `advert.delta[m]` | `m` 0..16. |
| `scope(27, k)` | `stack_obj.sprite_flags[temp[k]]` | `k` 0..=7. |
| `scope(30, k)` | `me.person[temp[k]]` | `k` 0..=7. `me` is the frame's object in every tree context, and not a BCON table here. |
| `scope(31, k)` | `stack_obj.person[temp[k]]` | `k` 0..=7. |
| `temp[J] := scope(24, i)` | `stack_obj.neighbor.<field>` or `.field[i]` | Read only, `i` 0..=9. The help names `age`, `friend_count`, `family_house` and `family_friend_count`, and uses `field[i]` for the others (the `ObjectId` and `bool` fields do not go into a `temp`). |
| `temp[J] := scope(32, i)` | `stack_obj.neighbor.person[i]` | Read only, `i` 0..=255. |

The scope cells of 12, 27, 30 and 31 count both as reads into `temp[0..7]` and as stores from a
literal, `temp[J]` or a `param[J]` below the argument count. Compound operators, `local[N]`
sources and other destinations stay raw.

The usual guards apply, tightened after review:

- Nothing is spelled when a local, parameter, Sim or event binding, user function, or `const`
  anywhere in the file takes the name the spelling uses (`temp`, `param`, `me`, `stack_obj`,
  `advert`, `reach`, `play_sound`, `clear_motive_changes`). Parsing substitutes a block constant
  for its name before the linter sees the body, so constants are read from the tokens.
- A template's member names no builtin call: it is built into objects in other files, and one
  of those may declare a member of the builtin's name, which would then take the call. Cells
  (`scope(..)`) name no call and are still spelled. This also applies to wave 1's
  `RAW_BUILTINS` forms, which now defer to a user function of the builtin's name as well.
- The block must end its last statement with `;`. `.air` reads adjacent raw blocks as one
  stream, so `raw { clear_motive_changes() } raw { pad(3: 0x05); }` is one padded primitive.
- Whitespace between two word tokens (`play_sound(1 2)`, `scope(1 2, 0)`) is never read as one
  token.
- Pads are checked by the encoder's own rule (`verify::apply_operand_pad`) against the call's
  plain encoding: ascending, each once, a new value, and only bytes the VM ignores. Clear Motive
  Changes encodes byte 3 as 1, so `pad(3: 1)` stays raw.
- The line filter that skipped any raw statement mentioning `pad` now lets a `pad` through only on
  `play_sound` and `clear_motive_changes`. Every other padded primitive still keeps its block.

### What stays raw, and why

- **`make_new_character`.** v2 reads skin, age and sex from named `i16` locals. A raw block names
  frame words (`local[N]`), and no v2 local is pinned to a given word, so no spelling builds the
  same node.
- **An unpadded `play_sound`.** Wave 1 left it raw on purpose, since v2 would rather name a
  declared `sound`, and `v2_lint.rs` pins that. Only the padded form, which has no other spelling
  than raw before #469, is new here.
- **Mode 1 `reach` in a functional's body.** The lint knows a tree's compiled argument count
  but not a functional's, so `reach(mode: 1)` there gets no hint (a false negative).
- **Scopes 28 and 29** (`advert.personality`, `advert.minimum`) were spelled before #469 and are
  not in this PR's brief. They are the same shape as scope 12 and can follow.

## Results

- **Open pack:** `lint --deny-warnings` finds nothing, `fmt --check` is clean, and the build is
  byte-identical to main (`diff -r` against `eb8d0ae3f`).
- **Corpus** (`cache/scratch/corpus-v2-behavioral`, 1,191 lifted files, each linted alone):
  0 errors or panics, and no finding of main's is lost.

  | Finding | main | this PR |
  | --- | ---: | ---: |
  | `manual_raw_has_builtin` | 577 | 2,492 |
  | `untyped_engine_field` | 5,971 | 6,006 (+35 `fame`) |
  | other rules | 12,381 | 12,381 |

  The 1,915 new raw hints break down as follows:

  | Help | Count |
  | --- | ---: |
  | `clear_motive_changes(padN: 0xNN)` | 739 |
  | `stack_obj.sprite_flags[temp[..]]` | 590 |
  | `me.person[temp[..]]` | 448 |
  | `stack_obj.neighbor.*` | 66 |
  | `play_sound(.., false, padN: 0xNN)` | 32 |
  | `advert.delta[..]` | 28 |
  | `reach(mode: .., moves: ..)` | 12 |
- **Every new hint applied.** A script rewrote each flagged block with its spelling and compared
  `check` and `expand` before and after, in 390 files.
  - 222 files checked before. All 222 check after.
    - 206 expand to the same `.air`.
    - 16 differ only where the optimizer, no longer blocked by the raw block (#469's scoped
      `keep_layout` pins a tree holding `scope(..)` or `pad`), drops a cursor store that the next
      statement repeats or merges identical branch tails. #471 saw the same thing.
  - 168 files fail `check` on main before any edit, so they could not be compared. They were
    lifted before #468, and their local `extern mod globals` declares one of the 45 new
    `std::globals` trees with another ABI. A re-lift fixes them; this PR does not touch them.

## Review

GPT-6.2 Sol (high effort), correctness and soundness only.

- **R1:** 5 findings. The 4 SHOULD-FIX are fixed, each with a regression test in
  `v2_lint_wave4.rs`: template members, block constants, a block that does not end its
  statement (and merged tokens), and pads the encoder refuses. The NOTE (functional frames) is a
  false negative, documented above.

## Consequences

- A `std` helper that one day takes a builtin's name is handled without a lint change.
- The lift's `untyped_engine_field` pass now prints `fame`.
- `manual_raw_has_builtin` gives Track 9 a list of lift targets: the pads, scope 12 and the
  temp-indexed and neighbor cells account for most of the 2,492 hints on the old behavioral
  corpus.
- Open: scopes 28/29 in raw blocks, and `make_new_character` if v2 ever pins a local to a
  frame word.
