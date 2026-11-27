# `.antics` v2.6 Step 1: writable `local[N]`, quarantined dead nodes, the loader check, clean-room global names

*Decided 2026-11-27.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-antics/src/node_order.rs` (the `DeadNode` API), `src/{ast,parser,lower}.rs` (the
    `.air` `dead_nodes:` header field)
  - `src/v2/{ast,parser,fmt,loader}.rs`, `src/v2/lower/{mod,lvalue,actions,trees,object}.rs`,
    `src/v2/std/globals.antics`
  - the slot-alias review fixes: `src/v2/lower/{air_text,builtins,stmt,match_lower}.rs`
  - new `tests/integration/v2_6_compiler.rs`; `tests/integration/{v2_typed_std,v2_lint}.rs`
    (renames), `tests/integration/v2_pr_g.rs` (`local[N]` writes now build)
  - no edits to `src/v2/{decompile,structure_lift,lint}.rs`, `open-pack/` or `docs/roadmap.md`
- **Track:** Track 8 (compiler), `.antics` v2.6 Step 1
- **Builds on:**
  - [`20261127-antics-v2-5-lift-finale.md`](20261127-antics-v2-5-lift-finale.md) (#509: the
    strict raw census, the `keep_layout` anchors);
  - [`20261126-antics-v2-5-compiler.md`](20261126-antics-v2-5-compiler.md) (#503: `keep_layout`,
    extern frame widths);
  - [`20261126-antics-v2-4-dead-false-error.md`](20261126-antics-v2-4-dead-false-error.md)
    (#498: `TreePin`, `match_tree_pin`, the wide-inline-call checks).

Every source, library name and tree name in the tests is synthetic. The corpus probes below
ran against the Track 9 lifts and are not committed.

## Decision 1: `local[N]` is writable, and a write keeps the layout as a read does

`local[N]` was read-only in v2 (#509 counted 3,722 strict and 2,762 behavioral raw writes it
forced). It is now a place:

- `local[N] = expr;`, every compound assignment (`+=`, `-=`, `*=`, ...), `dec_gt(&mut
  local[N], ..)` and the other mutable primitive operands, and `set_to_next(.., dest:
  local[N])`.
- **The layout.** A write goes through the same `read_frame_local_slot` as a read. It marks the
  raw span, sets the tree's `keep_layout`, and is refused beside a pinned
  `push_interaction(object: ..)` (`RAW_WITH_PIN` now names "`'local[N]'`", not just a read). The
  backend pass therefore never moves a named local onto a numbered word, and `.air` still
  places named locals after every explicit `local[N]`.
- **Discarded code** (`if false { .. }`) checks the write without keeping anything, as for a
  read.
- `fmt` needs no change: the tokens are those of an indexed assignment.

## Decision 2: nodes no branch reaches are quarantined as bytes (`dead_nodes`)

#509 found that 3,836 of the 4,961 strict raw-fallback trees keep nodes that no branch reaches
from node 0. 1,920 of them differ from their structured lift only by those nodes. A
structured body cannot produce a node nothing runs, so such a tree could only stay `raw`.

**The syntax.** A tree's body (a `#[bhav] fn` or a lifecycle block) may end in one block:

```text
#[bhav(id = 4200, node_order = [0, 3, 2, 1])]
fn t() {
    self.n = 1;
    ...
    dead_nodes {
        1: op 0x0002(0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04, 0x00) -> true: 2, false: error;
        4: op 0x0002(0, 0, 0, 0, 0, 0, 0, 0) -> true: 251, false: false;
    }
}
```

- Each entry is a node number, eight operand bytes and two branch bytes. A branch is `true`,
  `false`, `error` or any byte: nothing runs the node, so its bytes are kept even when they
  name a missing node or a reserved byte (251, 252).
- **Strictly opt-in, and never in reachable code.** The block is not a statement. The parser
  takes it only as the last item at the top level of a tree's body, followed by that body's
  `}`. Anywhere else (a nested block, a helper fn, before another statement) it is refused. An
  empty block is refused too. It goes into the tree's `TreeFrame` (`dead_nodes`), and lowering
  writes the `.air` header field `dead_nodes: [N: op .. -> true: T, false: F; ..]`. No
  statement, MIR pass or optimizer ever sees it.
- **`.air` assembly.** The body is built, then `dead_false`, `share_tails` and `node_order`
  are applied to its nodes as before (`node_order` counts the live nodes only). Then
  `insert_dead_nodes` puts each dead node at its number. The live nodes fill the remaining
  numbers in order, with their branches renumbered to follow them. `check_dead_nodes` refuses:
  - node 0, since the tree is entered there;
  - a number listed twice;
  - a number past the last node (live + dead);
  - more than 251 nodes in all.
- **Indexed trees refuse it.** A kind-4 tree enters at the node its first argument names, and
  inserted nodes would renumber them.
- **The wide-inline-call check holds.** A non-empty `dead_nodes` makes a tree "rewritten"
  (`rewrites` in `lower.rs`), so `check_inline_calls` / `narrow_callees` apply. In v2, a
  literal call to a callee not proven narrow is refused: "fn 't' has 'dead_nodes', which moves
  nodes, ...".
- **`fmt`.** Inside a `dead_nodes` block, a comma does not break the line and `op 0x0002(`
  stays tight. So each entry stays on one line, and formatting is idempotent.

**The API for the lift** (`node_order`):

- `split_dead_nodes(original)` returns the reached nodes, renumbered in order, and the rest as
  `DeadNode`s. It returns `None` when a reached node branches past the tree or to a reserved
  byte.
- `match_tree_pin_quarantined(compiled, original)` returns a `QuarantinedPin { pin:
  Option<TreePin>, dead_nodes }`:
  - with no dead nodes, it is `match_tree_pin`'s answer;
  - with dead nodes, the pin matches the reached nodes (`None` when they already match as
    compiled), with the dead nodes alongside.
- `dead_nodes_block(&dead, indent)` prints the block as `fmt` leaves it, and
  `DeadNode::entry_text` prints one entry.
- `TreePin` is unchanged: `decompile.rs` builds it as a struct literal.
- The round trip is exact: `insert_dead_nodes(split.live, split.dead)` rebuilds the original
  (`the_quarantined_pin_rebuilds_the_original` builds it through the compiler with a
  `node_order` too).

**Corpus probe (uncommitted).** In a local, uncommitted patch, `decompile.rs`'s `pin_nodes`
called `match_tree_pin_quarantined` and printed `dead_nodes_block` before the tree's closing
brace. It ran on the 40 strict-corpus objects with the most trees in #509's "same node count
once unreached nodes are out, same operands" class (918 of that class's trees, 5,193 trees in
all):

| `lift --v2` (strict) | identical | equivalent | inconclusive | different | idiomatic | raw |
|---|---:|---:|---:|---:|---:|---:|
| this branch | 852 | 728 | 3,613 | 0 | 3,766 | 1,427 |
| with the patch | 956 | 624 | 3,613 | 0 | 4,752 | 441 |

- 986 raw trees became idiomatic.
- No tree turned different, and no file failed.
- All 985 printed `dead_nodes` blocks pass `fmt --check`, and the files `check` clean.

## Decision 3: identical `if` arms need no new pin

#509 left 2 strict trees raw because their `if` has two identical arms, and the backend pass
merges those arms' nodes (`merge_duplicate_tails`). `keep_layout` (#503) already keeps the whole
backend pass off a tree, so the native `if` with `#[bhav(.., keep_layout)]` expands to the
original's `.air` in both trees (probe, uncommitted). No compiler change was needed, and so no
#509 anchor tree can change. `identical_arms::keep_layout_keeps_both_arms` pins the behaviour.

## Decision 4: the loader checks a file without reading it

`read_v2_checked_file` is now `check_v2_file` followed by `CheckedV2File::read`:

- `check_v2_file(path, pack_dir)` resolves the pack config and checks the path, with no read:
  - a regular file, and no symlink;
  - no symlinked component;
  - inside the pack root when `pack_dir` is given.

  It returns a `CheckedV2File { config, .. }` with `canonical_path()` and `read()`.
- `read()` reads through the same `read_regular_file_checked` against the same root as before.
- A linter that already holds a file's text gets the build's config, with the same path
  checks, without decoding the file again.
- `read_v2_checked_file` behaves as before for every caller.

## Decision 5: `std::globals` names are our own descriptions

Nineteen `std::globals` names sat close to descriptive labels, so they are renamed to
clean-room descriptions of what each tree does. The ids, signatures and parameter names are
unchanged, so every call assembles to the same bytes:

| id | name | id | name |
|---:|---|---:|---|
| 256 | `pick_own_image` | 320 | `on_downtown_lot` |
| 260 | `pick_stack_image` | 321 | `on_residential_lot` |
| 277 | `pick_group_image` | 366 | `pass_hours` |
| 278 | `claim_all_tiles` | 368 | `depart_lot` |
| 279 | `release_all_tiles` | 370 | `idle_with_motion` |
| 281 | `wait_until_interrupted` | 407 | `willing_to_train` |
| 286 | `display_skill_level` | 408 | `disabled_fun_gate` |
| 288 | `stack_object_busy` | 410 | `free_to_join` |
| 295 | `face_opposite_way` | 424 | `on_vacation_lot` |
| 297 | `report_needs_repair` | | |

- **Deferred:** 271, 272, 273 and 363. `decompile.rs`'s own tests spell those names, and that
  file belongs to Track 9, so they go with Track 9's next step.
- **Kept on purpose:** names that share only a generic word with any label.
- **Lifted files** that call an old name must be lifted again: the lift reads `std::globals`
  line by line to name calls. Their bytes do not change.
- open-pack calls none of the renamed trees.

## Verification

- Gates: `cargo fmt --all --check`, `cargo clippy -p sims-antics --all-targets -D warnings`,
  `cargo test --release -p sims-antics`, the runner (`open-pack/tests` +
  `crates/sims-antics/tests/test_runner`), `lint --deny-warnings` and `fmt --check` on
  open-pack, open-pack built byte-identical to `origin/main`, and
  `scripts/check_repository.py --history HEAD`.
- **Lift compatibility.** Every file of `lift-v2-5-final-strict` and `lift-v2-5-final-beh`
  was expanded with `origin/main`'s compiler, and again with this one after the old names in
  Decision 5 were replaced by the new ones (1,455 files call at least one). All 2,382 files
  give the same `.air`.

## Review

**Codex GPT-6.2 Sol, round 1** (correctness, soundness, regressions). Four findings, all
fixed with a regression test:

- **High: a `local[N]` write could alias an `#[slot(N)] let` local.** `#[slot(0)] let x = 7;
  local[0] = 1 + x;` was built in place (`local[0] := 1; local[0] += x`), and the second
  step read the word the first had just overwritten. The alias analysis
  (`canonical_air_cell`) now maps a slot local's air name to `local[N]`, so such a write is
  computed in a temporary first (`a_write_that_reads_its_slot_local_copies_it_first`).
- **Medium: a tail value before the footer** (`fn t() -> bool { self.n == 1 dead_nodes { .. }
  }`) gave "expected ';'". The block parser now treats the footer like the closing `}` for a
  tail expression (`a_tail_value_may_stand_before_them`).
- **Low: a composed lifecycle tree** (template block with `#[frame]`, object block with the
  footer) reported a second `#[frame]`. The footer is now merged apart from the frame
  attributes. A footer on two blocks of one tree is refused
  (`a_composed_lifecycle_tree_takes_one_footer`).
- **Low: `.air` took `dead_nodes: []` twice** without the duplicate-field error. Fixed.
- **Note (by design):** `match_tree_pin_quarantined` predicts the layout like
  `match_tree_pin`. The lift must still rebuild and compare, as it does today.

**Round 2.** Three findings, all fixed with a regression test:

- **High: a `#[slot(N)] let` initializer could overwrite its own input.** `local[0] = 7;
  #[slot(0)] let x = 1 + local[0];` was built in place, as `local[0] := 1; local[0] +=
  local[0]`. The same happened in an `if`/`match` arm that writes a slot binding, for an
  arithmetic chain and for a short-circuit `&&`/`||` chain. The path predates writable
  `local[N]`: a raw store reached it too. A slot binding's initializer, or an arm writing one,
  is now built in place only when it cannot read that word. Otherwise the value is built in a
  temporary and stored last. Other bindings are fresh locals, and their lowering is unchanged
  (`a_slot_binding_reads_its_word_before_writing_it`).
- **Medium: a tail `panic!()` before the footer** gave "expected ';'". It now ends a body as
  it does before `}` (`a_tail_panic_may_stand_before_them`).
- **Low: an empty `#[frame()]` on one block of a composed lifecycle tree** let another block's
  `#[frame(..)]` through. A frame made only for a footer is now marked (`footer_only`), so any
  written `#[frame]` counts. Two are refused again, as on `origin/main`.
