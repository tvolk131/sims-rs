# `lift --v2` wave 3: typed `std` spellings and PR I-4 forms

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/decompile.rs`; lift test expectations in
  `v2_fmt_and_decompile.rs` and `v2_lift_pilot.rs`; new `v2_lift_wave3.rs`
- **Track:** Track 9 (`lift --v2`), `.antics` v2.1 Wave 2 Stream 3b
- **Builds on:**
  - [`20261125-antics-v2-lift-wave2.md`](20261125-antics-v2-lift-wave2.md);
  - [`20261125-antics-v2-lint-wave3.md`](20261125-antics-v2-lint-wave3.md) (the three rules
    this change satisfies, and the temporary test filter it removes);
  - [`20261125-antics-v2-pr-i4-and-std-utils.md`](20261125-antics-v2-pr-i4-and-std-utils.md)
    (the cells, options and primitive slots this change starts writing).

## Context

On main after #471, lifted code did not use the spellings the lint asks for since #470:

- `untyped_engine_constant`: numbers that `std::flags` / `std::engine` name;
- `prefer_method_syntax`: `play_sound(s, x)` and friends where `x.play_sound(s)` exists;
- `prefer_question_mark`: `if !f() { return error; }` where `f` is a `std::globals` tree
  declared `-> Result`.

The lift tests ignored those three rules. #469 (PR I-4) also added v2 forms the lift did not
yet write, so their cells and primitives stayed in `raw { }`.

As in waves 1 and 2, nothing here changes what is claimed about a tree. Every spelling is
checked the usual way: the lift recompiles the file and proves each tree (strict or
`--behavioral`). A tree that a spelling breaks falls back as before.

## Decision 1: the lint's own suggestions, kept only when the bytes do not change

After the round loop and `type_engine_fields`, the lift runs the lint on its text and applies
the three rules' suggestions.

- **Reading a suggestion back.** The lint's edit ranges are private to `lint.rs`, so the lift
  rebuilds each range from the finding's position, and drops a finding whose text there does
  not have the rule's shape:
  - **constant:** the number literal at the column, which must equal the message's first
    quoted literal;
  - **method form:** the call name at the column, its `(`, and the balanced `)`. A string
    is skipped, and a comment refuses the finding;
  - **`?`:** `if !call { return error; }` or `if call {} else { return error; }`, and the
    suggestion must be `call?;`.
- **Imports.** A constant's help names the `use std::...;` line it needs. The lift merges
  those lines into the `use` block after the header comments, sorted, each once. If there is
  no block, it makes one.
- **The bytes gate.** The lint's wave 3 suggestions are each checked alone (see its ADR). The
  lift applies them together, so it builds the file again and keeps a set only when the
  file's `.iff` bytes are **identical** to the checked lift's:
  - The whole set is tried first.
  - A set that fails is split in halves. Each half joins what is kept only if the file still
    builds the same with both.
  - A file gets at most 24 builds per pass, and at most 3 passes. A rewrite can expose
    another finding: a method form can make a constant visible to the lint.
  - What fails or is not reached keeps its old spelling.

  Same bytes means every tree's proof carries over unchanged, in strict and behavioral mode.

## Decision 2: PR I-4's cells

These are written under a new **wave 3** switch, nested under wave 2. A tree a wave 3
spelling breaks is written again without wave 3 spellings, before it loses wave 2's or older
extras (`Plan::drop_wave3` runs first in every fallback chain).

| Cell | Written as | Where |
| --- | --- | --- |
| scope 27, `k` < 8 | `h.sprite_flags[temp[k]]` | `h` is `self` in a Sim tree whose stack object stays put, else `stack_obj`; not under a cursor (the lowering's cursor list has no sprite flags) or a Sims loop |
| scope 30, `k` < 8 | `sim.person[temp[k]]`; `me.person[temp[k]]` in an inherit tree | Sim and inherit trees |
| scope 31, `k` < 8 | `stack_obj.person[temp[k]]`; the binding's, `p.person[temp[k]]`, in a `for` loop whose cursor is the stack object | not in a Sims loop |
| scope 24, 0..=9 | `h.neighbor.field[i]` | as scope 27, or through a cursor binding |
| scope 32, 0..=255 | `h.neighbor.person[i]` | as scope 24 |
| scopes 12 / 28 / 29, `m` < 16 | `advert.delta[m]` / `.personality[m]` / `.minimum[m]` | any tree |
| person word 81 | `sim.fame` / `me.fame`, and the stack object's `stack_obj.fame` or a cursor or Sims-loop binding's `p.fame` | the lint's typed-word table does not name it, so the lift does |

- The `i16` neighbor words get their names: `age`, `friend_count`, `family_house`,
  `family_friend_count`, and the person words `posture`, `pets_outside`, `job_type`,
  `job_level`, `family`, `date_partner`, `fame`. The `ObjectId`, `bool` and enum ones keep
  the index, because a name would type their uses differently.
- Only a Sim tree can name `my.person[temp[k]]`: `self.person[temp[k]]` does not exist in
  an object tree. So scope 30 now counts toward choosing Sim context
  (`prefers_sim_context`).
- A Sim-context tree called from an object tree is tried as an inherit tree. There, a type
  test of the runner (`my.data[11]`) is written `me.id.is_type(..)`, as `self.id` and `sim.id`
  already were. Before, `me.data[11].is_type(..)` did not compile: the receiver is `i16`. The
  tree then fell back, and so did its caller's named call.

## Decision 3: PR I-4's options and primitive slots

- **`padN:` bytes.**
  - `play_sound` writes `padN: 0xNN` for each operand byte the pad changes outside the sound
    (bytes 0 and 1) and the flags (byte 4). Byte 4 is folded into `flags:` as before.
  - `clear_motive_changes` writes one `padN:` for each byte that differs from the lowering's
    own `[0, 0, 0, 1, 0, 0, 0, 0]`.
  - The pads are written only when wave 3 is on. Otherwise the older behavioral-only
    spelling is kept, which drops pads the VM never reads.
- **`reach`.** Mode 1 is written `reach(mode: 1, moves: B, slot: param[p])`. Other modes are
  written `reach(mode: M, moves: B)`, but only when their parameter operand is 0, since an
  omitted `slot:` encodes 0.
- **`make_new_character(skin:, age:, sex:, species:)`.** Written for species 0..=2. The three
  locals get named `i16` declarations (`slot_N`), even when nothing else reads them.
  - A pin that only this spelling needs marks the tree as using wave 3, so `drop_wave3`
    removes the pin along with the spelling.
- **`drop_onto`.** A parameter slot is written `param[p]`, on either side.
  - The same holds inside an animation handler. There the argument names the parameter by
    index, while the handler's `event` binding is a separate name for parameter 0.

## Decision 4: an `if` looping on itself keeps its `allow`

Lifting more cells uncovered loops the lift had kept raw. One is a comparison whose two
branches both go back to itself, written `loop { if dec_gt(&mut x, 0) {} }`. Like the
single-primitive self loop, it now carries
`// antics-lint: allow(unbounded_loop_without_yield): the original's node loops on itself`.
This happens only when the condition is a comparison (or its negation), which never yields,
so the `allow` cannot go stale.

## Results

The corpus is all 1,191 lifted objects, at 10 workers. "Main" is main after #471.

- **Gate:** both modes have 0 errors or timeouts, 0 DIFFERENT, 0 `fmt --check`-dirty files and
  0 `untyped_engine_field` findings.
- **Proofs:** every proven tree (Identical or Equivalent) is unchanged. Strict has 884 + 4,637;
  behavioral has 870 + 4,651.
- **Idiomatic trees:** unchanged. Strict has 8,503; behavioral has 25,308.

| | behavioral, main | behavioral, wave 3 | strict, main | strict, wave 3 |
| --- | ---: | ---: | ---: | ---: |
| inline raw statements | 17,165 | 14,920 | 3,190 | 3,059 |
| `raw {}` blocks | 12,563 | 10,820 | 19,538 | 19,436 |
| lines inside inline raw | 88,507 | 80,800 | 6,265 | 5,918 |
| total lines | 944,493 | 941,428 | 963,507 | 963,489 |
| `untyped_engine_constant` | 12,789 | **0** | 1,291 | **0** |
| `prefer_method_syntax` | 11,204 | **0** | 686 | **0** |
| `prefer_question_mark` | 213 | **0** | 91 | **0** |
| lint findings, all rules | 24,322 | 116 | 2,080 | 12 |
| sweep time | 74.6 s | 73.5 s | 83.3 s | 88.1 s |

- **What lint still reports** is unchanged from main: 116 behavioral (`manual_raw_has_builtin`
  64, `frame_headroom` 46, `negated_comparison` 3, `collapsible_if_chain` 2,
  `stale_lint_allow` 1) and 12 strict.
- **Raw statements per file.** In four behavioral files the count rises by one: one raw block
  splits in two around a newly lifted neighbor `if`, so the raw text there is shorter.
- **Sweep time.** The lint pass costs at least one extra build per file that has findings. The
  timings were taken on a loaded machine: across sweeps of this change, behavioral ranged
  from 65 s to 81 s and strict from 87 s to 98 s.
- **Strict mode** writes most trees whole as raw, so the PR I-4 forms mostly help in
  behavioral mode. Padded calls that stay raw (about 2,500 `play_sound` and 640
  `clear_motive_changes` in strict, 414 and 240 in behavioral) sit in raw-fallback trees, or
  in trees that some other spelling already pushed below wave 2. An example: the spelled
  form builds there, but the tree has lost every extra.
- **New spellings in behavioral output:**
  - `padN:` options on 2,871 `play_sound` and 629 `clear_motive_changes` calls;
  - 687 `sprite_flags[temp[k]]` and 504 `person[temp[k]]`, most of them in inherit trees;
  - 213 `advert.delta[m]`;
  - 512 neighbor cells, most of them named;
  - 112 `fame`;
  - 166 `reach(..)`, 196 `drop_onto` parameter slots and 13 `make_new_character(..)`.

## Consequences

- `lift_source` in `decompile.rs`'s tests asserts lint-clean output under **every** rule
  again. Track 7's temporary filter is gone.
- Lift test expectations that pinned the older spellings now expect the new ones:
  - `stack_obj.play_sound(..)`, `sim.play_sound(..)`, `me.play_sound(..)`,
    `stack_obj.stop_all_sounds()`, `x.push_interaction(..)`;
  - `ObjectFlags::InUse`, `ObjectFlags::FloatsOnWater`, `Height::Floor`,
    `WallAdjacency::WallFront`;
  - `drop_onto(src: param[0], dst: 2)`, which no longer needs `raw { }`.
- **`from_raw` folding (#469).** No lifted file contains `from_raw`. Comparing the
  pre-#469 lift with main shows 0 proof regressions and 0 DIFFERENT. Two strict-mode trees
  (two alarm objects) did stop being idiomatic, but from another #469 change: the raw-block
  optimizer now hoists an `if`'s identical arm tails out of a `raw { }`, which changes the
  bytes. Strict mode then writes the whole tree raw. Behavioral mode is unaffected.
  - **Follow-up for the lowering owner:** keep a `raw { }` block's arms as written, or have
    the lift spell such arms joined.
- **Follow-up for Track 7:** `untyped_engine_field` does not know `fame` (person 81), so the
  lift names it itself.
- **Not done:** `make_new_character` in an `init` or `main` block stays raw. The lift does
  not pin locals in lifecycle blocks, and no corpus object has the call there.
