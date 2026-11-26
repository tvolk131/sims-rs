# `.antics` v2.2 step 2: `wall_placement` and `world.expansions` take flag methods; `return error;` retires

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/{ast,parser}.rs`, `src/v2/lower/*`, `src/v2/lint.rs`
  and `src/v2/decompile.rs` (Tracks 7 and 9 were idle; this PR owns both files for the step).
- **Track:** Track 8 (compiler), `.antics` v2.2 polish wave, step 2a
- **Builds on:** [`20261125-antics-v2-2-temps-and-flags.md`](20261125-antics-v2-2-temps-and-flags.md),
  [`20261125-antics-v2-panic-and-discard-let.md`](20261125-antics-v2-panic-and-discard-let.md)

## Context

- Two engine flag words were still plain `i16` to lowering:
  - the object word `wall_placement` (`data[13]`, `std::flags::WallPlacement`);
  - the world word `world.expansions` (`global[20]`, `std::flags::Expansions`).

  So `.set` / `.contains` were refused on them. Lifted code kept the raw operators
  (`flag_set` / `flag_clear` / `flag_test`) for every behavioral flag operation on these two
  words, about 2,795 of them across the corpus. Lint could not suggest the method form either.
- `return error;` and `panic!()` were two spellings of the error exit. `return error;` also had
  a special case: with a binding named `error` in scope, it returned that binding instead.
  Since #475 the lifter writes `panic!()`, and lint suggests it.

## Decision 1: `wall_placement` and `world.expansions` are lenient engine `bitflags` words

- **`wall_placement`** is typed `std::flags::WallPlacement` in `OBJECT_DATA_WORDS`
  (`lower/tree.rs`). It behaves like `wall_adjacency`, `flags2`, `placement`,
  `allowed_heights` and `buy_flags`, on `self`, a Sim, `stack_obj` and handle locals.
  - Written by number, `self.data[13]` stays `i16`, as every numbered data word does.
- **`global[20]`** is typed `std::flags::Expansions` (`global_word_type` in
  `lower/lvalue.rs`).
  - The parser reads `world.expansions` as `global[20]`, so the two spellings are one cell
    with one type, and lint already treated them that way.
  - It stays writable, as it was.
  - Every other world word stays `i16`.
- **Methods.** `.set`, `.clear`, `.contains` and `.intersects` work, as do the list forms
  `set_all`, `clear_all`, `contains_all` and `contains_any`. They lower to the same nodes as
  the raw operator with the same bit; the tests compare the `.iff` bytes.
- **Lenient means the words keep their `i16` interop.** Each of these builds, with the
  bytes the same source has with `self.data[13]` / a plain `i16` world word:
  - raw bit numbers on the operators (`flag_set 12`, `flag_test 3`) and comparisons with
    numbers (`== 0`, `> 3`);
  - assignment from and to `i16` cells, arithmetic, and an argument to an `i16` parameter;
  - `match` with integer arms. `match` now treats a lenient engine flag scrutinee as `i16`.
    It refused a bitflags scrutinee before, and this word was `i16` until now;
  - compound operators taking an `i16` or a word of the same type (`w += 3`,
    `W += W`), on the word or on a local holding it;
  - a numeric `run_functional` receiver (`let w = world.expansions; w.run_functional(4);`);
  - an inline helper's `&mut i16` parameter (`bump(&mut self.wall_placement)`), which sees
    the cell as `i16`;
  - builtins that ask for an `i16`:
    - `random_into` (place and range);
    - `inc_lt(&mut x, n)`-style steps;
    - relationship `value:` (the number and the output cell), slot parameters, range bounds
      and `job_data` operands;
    - the `sprite_flags` index, the `set_dynamic_sprite` value and `T::from_raw(..)`;
    - a `set_to_next(dest:)` local.

    These now go through one helper, `i16_like` (`lower/mod.rs`), which takes `i16` or a
    lenient engine flag word. Before, each one took exactly `i16`, which already refused
    `wall_adjacency` and the other engine words there.
- **Kept strict: the rules every typed engine word (`flags2`, `wall_adjacency`, ...) already
  has.** Each of these built while the words were `i16` and is now refused, as it is for
  `flags2`. None of them appears in open-pack, the test corpora or lifted output; open-pack
  builds byte-identically. Write `self.data[13]`, or `world.expansions as i16`, where the
  plain number is meant (`global[20]` written out is the same typed cell).
  - A flag *constant* of another type (`self.wall_placement flag_set WallAdjacency::WallFront`,
    `world.expansions.set(WallPlacement::NoCorner)`). It is a likely bug where it is written.
  - A runtime value of the word as a bit number (`temp[0] flag_set world.expansions`,
    `temp[0] flag_test self.wall_placement`, or through a local): a typed flags value is a
    mask, never a bit number (`flag_operations_never_take_a_mask_as_a_bit`). The left operand
    of `flag_test` was already lenient for engine words; the right one stays `i16` only.
  - Mixing with an `ObjectId` (`let o: ObjectId = world.expansions;`,
    `world.expansions = stack_obj;`, `w -= stack_obj`).
  - `& | ^`, ordering comparisons and assignment between two engine words of different
    types (`self.wall_placement & world.expansions`, `world.expansions < self.flags2`).
    Arithmetic (`+ - *`) between them still builds, as `i16`.
  - `if` / `match` value branches that join the word with an `i16`
    (`if c { world.expansions } else { 0 }`): branches must agree on one type, and a lint
    test relies on that rule for `flags2`.
- **No `HouseParty` bit.** `std::flags::Expansions` lists only the bits whose meaning is
  confirmed (Dating, Vacations, Pets, Fame, Magic). The brief's example is written with
  `Pets` / `Magic` instead.

### Lint (`prefer_method_syntax`)

- `untyped_engine_field` no longer proposes `self.data[13]` → `self.wall_placement`
  unconditionally (the `i16` table). It goes through the byte-checked trial for the typed
  words, as `flags` does: a cross-type constant on `self.data[13]` builds, but would not under
  the typed name.
- The rule's carve-out for `wall_placement` is gone: `self.wall_placement flag_set WallPlacement::X`
  now suggests `self.wall_placement.set(WallPlacement::X)`.
- `world.expansions flag_test Expansions::X` suggests `world.expansions.contains(Expansions::X)`.
  The parser gives the desugared `global[20]` the span of the `.`, so the rule checks that the
  source reads `world.expansions` there.
  - `global[20]` written out gets no method suggestion. `untyped_engine_field` already asks
    to name it `world.expansions` first.
  - A spaced `world . expansions` gets no suggestion either (the check reads the source
    text). It is rare, and the operator form stays valid.

### Lift

- `decompile.rs`'s polish pass already tries the method form for each place and keeps it when
  the build is byte-identical. The pass is unchanged; `world.expansions` and `wall_placement`
  now pass that check, so their typed operators lift as methods. Only a comment changed.

## Decision 2: `panic!()` is the one spelling of the error exit

- The parser no longer turns `return error;` (statement and bare `match` arm) into the error
  exit. It parses as `return <expr>;` with the identifier `error`, marked `bare_error` on
  `V2Stmt::Return` so lowering knows it is that exact spelling (spans carry no byte offsets,
  so lowering cannot tell it from `return (error);` otherwise).
  - **With a binding named `error`** (a local, a parameter or an object parameter), it
    returns that value, exactly as before.
  - **Without one**, lowering refuses it with a message that names the replacement:
    `unknown identifier 'error': 'return error;' is no longer the error exit; write 'panic!();' (or bind a value named 'error' to return it)`.
    Any other read of an unknown `error` (an inline helper's tail condition, an operand)
    says `unknown identifier 'error' (the error exit is 'panic!();')`.
  - **With only a `const error`**, it is refused too. The old binding check did not count
    consts, so `return error;` was the error exit there; returning the const would silently
    swap the exit for a value. The message asks for `panic!();` or another name.
  - **`return (error);` and a tail `error`** were never the exit and are untouched: they
    return the binding or the const, with their old bytes.
  - An inline helper ending in a bound `return error;` keeps the lowering it had as the
    error-exit statement: `has_single_inline_tail_return` skips the `bare_error` spelling
    (only), so its bytes are unchanged.
- `V2Stmt::ReturnError` loses `from_try`. It is now only `panic!()` (`panic: true`) or the
  false edge of `cond?;` (`panic: false`). A `?` inside `defer` is refused as
  `'?' cannot exit out of a 'defer' block`, as before.
- **`raw { }` is `.air`.** `return error;` there is `.air`'s error exit and is unchanged, as
  is the `.air` lowering emits.
- **Lint:**
  - `prefer_question_mark` matches only `panic!()`. A `return error;` branch is a return
    of a value, so it gets no `?` suggestion.
  - `unreachable_code` says "`return`" for it.
  - The binding check is gone: `panic!()` never names a binding.
- **Decompile:** `is_error_exit` (used when applying lint's `?` rewrites) accepts only
  `panic!()`.
- `stmts_have_return` still counts the error exit as a return (it is conservative for
  inlining helpers with a single tail return), so inlined bytes do not change.
- **Fixtures:**
  - every v2 test source in `tests/integration/v2_*.rs` and the unit tests now writes
    `panic!();`;
  - tests that compared `panic!()` against `return error;` now check the error exit in
    the `.air`, or compare two `panic!()` spellings;
  - `.air` expectations and `raw { }` bodies keep `return error;`.

## Consequences

- open-pack: byte-identical to main; lint and `fmt` are clean.
- `return error;` without a binding named `error` is a compile error. Every v2 source in the
  repository had already moved to `panic!()`, apart from test fixtures, which this PR
  migrates.
- New tests: `tests/integration/v2_2_bitflags_and_return_error.rs`. They cover:
  - byte identity of each method and list form against the raw operator;
  - the Sim, `stack_obj` and handle-local receivers;
  - every `i16` use, compared byte for byte against `self.data[13]` / a plain world word;
  - the refusals: flags of another type, runtime bit numbers, `ObjectId` and cross-type
    mixing, and value branches with an `i16`;
  - lint suggestions and their fixes;
  - the lifter writing the methods, in strict and behavioral mode;
  - the `return error;` diagnostic, `let error = true; return error;`, and raw `.air`.
- Pre-existing, not changed here (Track 8 follow-up): a flag method on an object-loop
  handle (`for o in lot.objects() { o.flags2.set(..); }`) is not recognized by the direct
  cursor lowering (`cursor_stmt_ok`, `lower/stack.rs`), so it saves `stack_obj` and needs a
  local where the operator form steps the cursor directly. It already held for `flags2` and
  the other typed words; now it holds for `wall_placement` too. Neither lint (it suggests
  methods only on `self`, a Sim and `stack_obj`) nor the lift (byte-checked) writes that form.
- Track 9 follow-up: re-measure the corpus idiomatic share. The behavioral flag operations
  on these two words should now lift as methods.
