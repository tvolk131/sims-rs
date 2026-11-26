# `.antics` v2.2: flag lists, zero-copy temp arguments, `Option<T>` answers

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/{ast,parser,loader}.rs`, `src/v2/lower/*` and
  `src/v2/std/globals.antics`. A small tolerance in `v2/decompile.rs`'s std readers
  (see Consequences). No change to `v2/lint.rs`, `v2/fmt.rs` or the optimizer.
- **Track:** Track 8 (compiler), `.antics` v2.2 polish wave, step 1
- **Builds on:** [`20261125-antics-v2-panic-and-discard-let.md`](20261125-antics-v2-panic-and-discard-let.md)

## Context

Three costs and gaps remained in everyday code:

- **Several flags at once.** Setting or testing two bits needed a union written out:
  `self.flags.set(ObjectFlags::A | ObjectFlags::B)`. A list reads better.
- **Arguments copied for nothing.** Every call argument that read a temp was first copied into
  a fresh local, because the temps are overwritten while the arguments are stored. A call
  passing `temp[0]` as its first argument paid a local and two nodes to store `temp[0]` into
  itself. The lifter emits exactly this shape for trees that pass their temps through.
- **Answers that carry a value.** Globals 282 and 333 return true with a value in `temp[0]`
  and false with nothing. They were typed `-> bool`, and callers read `temp[0]` by hand after
  the test.

## Decision 1: `set_all`, `clear_all`, `contains_all`, `contains_any` take a list

- The four methods take one bracketed list of flags: `self.flags.set_all([A, B])`. A trailing
  comma is accepted. A turbofish (`clear_all::<i16>([..])`) parses but is refused, as type
  arguments are on every flag method.
- A list holds at most 64 flags, the parser's nesting limit: the fold nests one level per flag,
  as a written-out union does.
- The list exists only as the argument of these methods. The parser flattens it into
  ordinary call arguments, so there is no new expression node and no `lint.rs` arm. A bracket
  anywhere else is still a syntax error.
- Lowering folds the elements into `A | B | ..` and hands the result to the existing
  `set` / `clear` / `contains` lowering. `contains_any` is the `intersects` lowering. The bytes
  are the union form's bytes by construction. A single element is one bit operation, exactly
  like `set(A)`.
- Type checks are the union form's: every element must be the receiver's flag type. An `i16`
  element, a flag of another type, or a receiver that is not a bitflags value is refused.
- **The empty list is an error** (`'.set_all([])' names no flags`). It would be a silent
  no-op for `set_all` and `clear_all`, and a constant (true or false) for the predicates.
  Neither is what anyone writes on purpose.
- Missing brackets, two lists, and a list on any other method are refused with a message
  naming the list form.
- `fmt` prints the list as written. A trailing comma prints as `, ]`, as it already does
  inside parentheses.

## Decision 2: a call whose arguments are temps and literals stages them in place

A call's arguments are stored into the callee's temps. The copying form (unchanged, and still
the default) snapshots into a fresh local every operand that reads a temp, that a later
argument could change (`later_impure`), or that may fault (`may_fault`). It then stores every
operand in parameter order. Now:

- **A call whose every argument is a literal or a bare `temp[k]` (`k < 8`) stages in place**
  (`stages_in_place` and `emit_temp_moves` in `lower/callable.rs`). No such operand can fault
  or run code. A call with any other argument (a local, an attribute, arithmetic, a call)
  keeps the copying form. Its reads may fault (an absent stack object's attribute), and the
  temps and last result a fault leaves must be the copying form's.
- **In place, the stores keep parameter order, and each reads its pre-staging value:**
  - before a store overwrites a temp a later store still reads, that temp is saved into a
    fresh local, which the later store reads instead. A swap (`f(temp[1], temp[0])`) costs one
    local, as before;
  - a store of a temp into itself is dropped, **except the last store**. The staging's final
    Expression sets the last result and the scratch pointer the callee sees (`global[34]`, in a
    library tree whose body the compiler cannot inspect). Keeping the copying form's last store
    leaves both exactly as before. Every earlier store's result and pointer are overwritten
    by the next node, since nothing between can fault or read them.
- **The copying form stays everywhere node identity is observable:**
  - **The object may read global 34** (`ObjectEnv::reads_global_34`). Every store is an
    Expression, and a call carries the scratch pointer into the callee. The object is lowered
    again when a tree turns out to read it, as for the optimizer's main rules.
  - **Inside a `#[temp]` loop** (`temp_loops`). The loop's counter check reads the body's
    `.air` for writes to the counter, so every store must be there. This includes calls in a
    `defer`, a nested loop, or an inline helper's captured branch.
  - **In a tree with an explicit `kind`** (`fixed_entry`). An indexed tree is entered at a
    numbered node.
  - **Where the tree's nodes or frame words are observable.** A tree's lowering records that
    it staged in place (`staged_in_place`). `finish` then sets `ObjectEnv::saw_layout_staging`
    if any of these holds, and the object is lowered again with `copy_staging`:
    - the tree keeps its layout (a raw block, a local slot, a wide literal call): numbered
      raw branch targets count nodes;
    - the tree takes main's rules (global 34, a bottom frame read before it is written, a
      pin);
    - the tree's text may name frame words (`$Local:N` reads whichever local lives there);
    - the tree may be a bottom frame (an argless object tree) and is left with no frame word
      at all. Create Object with flag 0x10 writes word 0, which faults on an empty frame.
  - **In an object one of whose trees keeps its layout.** That tree may read a frame word
    another tree's call left there (`local[N]` in a raw or slot read), so no tree of the
    object changes its frame (`saw_in_place` with `saw_keep_layout`).

  These facts are only known once a tree is lowered (a raw block or a pin can follow the
  call), so a second lowering decides them, as for global 34.
- **Residual assumption**, the one the optimizer already makes: a tree outside the object
  (a global or semi-global) does not read a frame word that an object tree's call left
  behind, before writing it.
- **Effect:** `f(temp[0], temp[1])` into slots 0 and 1 is one store (`temp[1] := temp[1]`) and
  the call, with no local. A tree with `#[bhav(locals = 0)]` can pass its own arguments on. A
  lifted `boil(temp[0], temp[1])` needs no frame locals. All-literal calls keep the literal
  call encoding. The gain is narrower than "every bare temp": an argless object tree whose
  only locals were the copies keeps them (it may be a bottom frame), and so does every
  object with a raw block, a local slot read or a global 34 read.

### `#[temp(N)]` in `std::globals`: no change

The brief asked to check `#[temp(N)]` signatures for 260, 285, 286, 302, 405, 408 and others.
None of them changes in this PR:

- With Decision 2, calling one with a positional argument that is already in its temp
  (`globals::wants_fun(temp[0])`) passes it with no copy: the temps call (`call 285(temps)`)
  after the one store the copying form ends with. The `#[temp(N)]` form would buy nothing at
  these call sites.
- The decompiler reads `std/globals.antics` with a line parser (`std_global_table`) that
  splits at the first `)`. It would drop entries written with `#[temp(..)]`. That changes which
  local declarations lifted code emits and can make them clash with std on merge.
- For a mixed tree, the literal-site encoding would change. Global 260 alone has 365 literal
  call sites.

A `#[temp(N)]` change waits until the decompiler's table parser reads attributes (Track 9).

## Decision 3: `#[ret(temp = N)] -> Option<T>` is an answer that carries a value

- **Declaration:** `#[ret(temp = N)] fn f(..) -> Option<T>;`, on `extern mod` functions and
  `#[bhav]` trees. `Option<T>` without `#[ret(temp = N)]` is refused, with a message asking
  for it.
- **Encoding:** the AST reuses `ReturnAbi::Value { ty, temp, option: true }` rather than a new
  variant. `lint.rs` matches `Value { .. }` and treats it as a value return, which is right for
  everything it checks, so it needs no edit. Lowering resolves it to
  `ResolvedReturnAbi::OptionValue { ty, temp }`.
- **Call sites.** The tree is called exactly like a `-> bool` one. The true exit means `Some`
  and the value is in `temp[N]`.
  - `if f() { .. }`, `while`, `!f()`, `&&` / `||`, and a bare statement `f();` behave as for
    `-> bool`.
  - `f()?;` propagates the false exit as the error exit.
  - `let v = f()?;` unwraps: the error exit on false, otherwise `v` is `temp[N]`.
  - `let Some(v) = f() else { .. };` runs the `else` block on false, which must diverge,
    then binds `v` to `temp[N]`. An optional type annotation on the binding
    (`let Some(v: T) = ..`, the existing `let Some` spelling) is checked.
  - `let x = f();` is refused: there is no `Option` value to hold. The message names the three
    forms above.
- **Zero hidden overhead.** The value is the raw `temp[N]` read, exactly what hand-written
  code read. A use the optimizer forwards costs no local: `self.n = k;` after
  `let Some(k) = take_elapsed_periods(30)` is the call node, the false branch and one store.
  A compound use (`self.n += k;`) keeps one local, exactly as `let k = temp[0]; self.n += k;`
  does today. `forward_copies` does not forward into compound operators. That is an existing
  optimizer limit, left alone so open-pack stays byte-identical.
- **Bodies.** A tree declared `-> Option<T>` returns `Some(v)` (stores `v` into `temp[N]`,
  then the true exit) or `None` (the false exit), as a `Result<T>` tree returns `Ok(v)` and
  `Err`.
- **Merging declarations.** A local `-> bool` declaration of the same tree still merges with
  the std `-> Option<T>` one: `same_extern_abi` treats the two as the same calling
  convention. Two `Option` declarations of one tree must agree on the type and the temp.
  Two declarations under the same name must agree exactly (`b.ret == f.ret`), so the merge
  does not depend on which file comes first. Aliases within one block stay free, as they are
  for every return ABI: a block may declare one tree as `-> Option<i16>` with temp 0 and with
  temp 1, as it may as `-> i16` with temp 0 and with temp 1.
- **`if let Some(v) = f() { .. }` is deferred.** `if let` needs walker support in `lint.rs`
  (Track 7). `let Some .. else` covers the common case today.
- **std:** 282 is `#[ret(temp = 0)] fn take_elapsed_periods(period: i16) -> Option<i16>`,
  333 is `#[ret(temp = 0)] fn find_companion() -> Option<ObjectId>`.

## Consequences

- **Track 9's test file changed.** `v2_lift_wave2.rs`,
  `temps_calls_to_trees_with_arguments_lift_in_behavioral_mode`: the lifted tree now has
  `locals = 0` (was 2). This is Decision 2 working. It is a one-line expectation change.
- **`v2/decompile.rs` (Track 9's file) got a small tolerance.** Its std readers
  (`std_global_table` and `std_global_names`) skip a `#[ret(temp = 0)] ` prefix, and the table
  reads `-> Option<..>;` as an answer, like `-> bool;`. Without it, 282 and 333 drop out of the table, the lifter stops
  seeing their std ids, and an object that declares one of them another way fails to lift
  (`extern mod 'globals' calls tree 282 two ways`) where it lifted into `more_globals` before.
  Lifted calls stay in bool form (`if globals::take_elapsed_periods(30) { .. temp[0] .. }`),
  which the `Option` form accepts with the same bytes. Track 9 follow-up: lift
  `let Some(v) = .. else`, and read `#[temp(N)]` attributes.
- **Track 7 follow-ups:**
  - add `set_all`, `clear_all`, `contains_all` and `contains_any` to `NON_YIELDING_METHODS`;
  - add `if let` to the walker, so the compiler can accept `if let Some(v) = f()`;
  - lint suggestions for the list form and for `let Some`.
- `return error;` is still accepted.
- open-pack: byte-identical, lint and `fmt` clean.
- Tests: `tests/integration/v2_2_temps_and_flags.rs` (byte identity of every list form with
  its union, node and local counts for staged calls and `Option` answers, the copying form
  under global 34 and kept layouts, the loop counter, misuse messages, merge rules).
