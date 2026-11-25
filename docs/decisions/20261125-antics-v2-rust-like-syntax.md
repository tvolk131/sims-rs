# `.antics` v2: Rust-like syntax (PR I-1)

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/**`, `crates/sims-antics/scripts/migrate_v2_syntax.py`, the
  test sources, `open-pack/`
- **Track:** Track 8 (v2 front end and lowering)
- **Follows:** `20261124-antics-v2-pr-h-specialization-and-cleanup.md`,
  `20261124-antics-v2-r3-lowering-fixes.md`

## Context

`.antics` v2 grew its surface one PR at a time: `tree` items, `@id(..)` / `@frame(..)` / `@slot`
annotations, `name @ N` addresses, statement-only `if` / `match`, raw cell numbers for engine
data, and object files that copy the same `extern mod globals { .. }` block. A Rust reader has to
learn a second notation to read it.

The language should read as idiomatic Rust: no `tree` keyword, no `@` sigils, expression-oriented
control flow, named engine fields. It must stay a pure surface change:

- every new spelling lowers to exactly the nodes its long form does;
- no hidden locals and no extra nodes;
- `lift --v2` stays at 0 DIFFERENT.

## Decision 1: attributes and `fn` replace `@` annotations and `tree`

| Before | After |
| --- | --- |
| `@id(4096) @frame(args = 4) tree main_4096 "Main" { .. }` | `#[bhav(id = 4096, label = "Main", args = 4)] fn main_4096() { .. }` |
| `@id(4096) main { .. }` | `#[id(4096)] main { .. }` |
| `state x @ 2: i16;` | `#[slot(2)] state x: i16;` |
| `@slot(3) let x = ..;` | `#[slot(3)] let x = ..;` |
| `entry @ 16521;` / `entry @ 257[3];` (extern bcon) | `#[addr(16521)] entry;` / `#[bcon(257, 3)] entry;` |
| `fn f(level: i16 @ temp[0])` | `fn f(#[temp(0)] level: i16)` |
| `fn f() -> i16 @ temp[0]` | `#[ret(temp = 0)] fn f() -> i16` |

- Attributes are parsed by `v2/attrs.rs`.
- Each item names the attributes it accepts, so a misplaced attribute is an error at its own span.
- The old forms are errors that name the replacement. The `tree` keyword error, for example, says to write `#[bhav] fn`.
- `scripts/migrate_v2_syntax.py` rewrites old sources. Every test source and `open-pack/` were migrated with it.
- `fmt`, `lint` and `lift --v2` read and write only the new syntax.

**Default `main` (#446).** An object with no `main` now gets the idle loop `while true { idle(1); }`, not `return true;`. The migrator gives every object that relied on the old default an explicit `main { return true; }`, so migrated objects keep their bytes.

## Decision 2: expression-oriented `if` / `match`

**Diverging branches.** A branch of `let x = if ..` or `let x = match ..` can leave instead of yielding a value:
- `return ..;`, `break;` or `continue;` (bare in a `match` arm);
- or a block that leaves on every path.

```rust
let step = match self.n { 0 => break, 1 => continue, _ => self.n };
let v = if self.n > 3 { self.n - 3 } else { return false; };
```

- Lowering writes the leaving branch where the statement form would.
- A nested `else if` / `match` chain may leave on every branch; the enclosing chain supplies the value.
- A leaving branch makes the lowering skip the single-pass `while true` it uses for `||` / `&&` and for match chains, so `break` / `continue` exit the user's loop.
- A block branch that can fall through is an error: "must leave on every path". A `raw { .. }` block that may jump out of it (a `break`, or a branch to a label or node number) is an error, because the leaving analysis cannot follow it. A raw `continue` is fine.
- So is a block branch outside a `let` initializer: "only be a branch of 'let x = if ..'".

**Tail expressions.** In a function that returns a value, the last expression can omit its `;` and becomes `return <expr>;`. The tail can also sit in each branch of a final `if` / `match`, either braced or as a bare arm without `;` (`if a > b { a } else { b }`, `match a { 0 => 1, _ => 2 }`). An arm ended by `;` is a statement. A tail after `const X = if ..` ends every specialized copy. This covers:
- `fn .. -> T`;
- `#[bhav] fn` with a result;
- guard blocks.

A tail anywhere else is the error "expected ';' after this expression".

## Decision 3: typed engine fields

Named fields read and write the raw cells. Each name below was checked against sims-vm's own use of the cell.

| Spelling | Cell |
| --- | --- |
| `world.hour`, `.day`, `.selected_sim`, `.time_of_day`, `.minute`, `.second`, `.month`, `.year`, `.house`, `.paused`, `.days_played`, `.free_will` | `global[0, 1, 3, 4, 5, 6, 7, 8, 10, 17, 28, 30]` |
| `self.room_impact`, `self.wall_placement`, `self.broken` | object data `7, 13, 15` |
| `sim.posture`, `sim.pets_outside`, `sim.job_type`, `sim.job_level`, `sim.family` | person data `0, 34, 56, 57, 61` |

- `world` is reserved: it cannot name a binding (`const` bindings included), an import, an `extern bcon` or a BCON table.
- An unknown `world.x` is an error.
- Fields whose meaning we could not confirm in sims-vm were left out: graphic, job performance, job day, animation events, and cheats (global 20 is the expansions mask).
- `lift --v2` writes `world.<name>` for the named globals.

## Decision 4: methods

- **Sound and refresh.** `obj.play_sound(s)`, `obj.stop_all_sounds()` and `obj.refresh(mode)` are the call forms with `target: obj`.
- **Funds.** `sim.debit_funds(n, category: c)` and `sim.credit_funds(..)` are `alter_budget`. The receiver must be the interaction's Sim.
- **Already supported.** `.set()`, `.clear()`, `.contains()` and `push_interaction` were already methods.
- **Lint.** The new methods are in the lint's non-yielding list.

## Decision 5: sugar

**`use std::globals::*;`** A built-in, clean-room module: `pub extern mod globals { #[id(N)] fn .. }` with 17 commonly used global trees, for example:
- `init_object`, `standard_entry`, `standard_exit(delay)`;
- `idle_ticks`, `wait_for_notify`, `animated_idle_for`;
- `ensure_standing`, `am_i_a_child`, `is_residential_lot`.

Notes:
- The names and comments are our own descriptions; only the tree ids are engine facts.
- Actions return `Result` (for `?`); questions return `bool`.
- The source is `src/v2/std/globals.antics`, compiled in with `include_str!`.
- A glob import brings in every `pub` item; local items shadow it.
- An unknown `std::` module is an error.
- **`std::bcon` is deferred.** We have no clean-room naming for the standard BCON slots that we could check, and inventing names would guess at semantics.

**`guid: CONST`.** An object or variant `guid:` takes a `const` name. It is resolved during lowering like any const: object/template-local consts first, then module-level and imported ones. Unknown names and values outside the u32 range are errors.

**`Prices::snack`.** A path on a BCON table (private or `extern bcon`) lowers exactly as `Prices.snack` does. That covers values, conditions, constant evaluation, trap analysis and stack-cursor checks. An enum or bitflags type with the same name keeps the path.

**`sound s = fwav(N[, wav: "name"]);`** Replaces `preset("ui_click", [wav: "name",] id: N)`. It gives the same `SoundDecl` (default preset, explicit FWAV id), and `lift --v2` writes it.

**`const` parameters on inline `fn` helpers.** A call can already pass a constant to a read-only parameter, and the parameter is then bound as that constant. `const k: i16` makes this a checked contract:
- every call must pass a compile-time constant (an i16, bool, enum or bitflags value);
- the helper may not assign or borrow `k`.

So a body that specializes on `k` (`const X = if k == 1 { .. } else { .. };`) can never fall back to a runtime test. Other rules:
- a constant is a literal, a `const` item, an enum or bitflags value, or another `const` parameter;
- a tunable `param` is a run-time BCON read, so it is refused;
- the language has no constant arithmetic, so `1 + 2` is refused (as in `const X = 1 + 2`);
- functionals cannot take `const` parameters;
- the type must be i16, bool, or an enum or bitflags type.

**`#[temp(N)] for i in a..b`.** A range loop whose counter is `temp[N]` itself, the way hand-written trees count (`temp[0] := 0; .. inc_lt`):
- It lowers to the same nodes as the ordinary range loop, with stores to `temp[N]` where that loop declares a frame local. No frame local is used.
- The loop's own steps are spelled `scope(8, N) += 1;`. That is the same operand as `temp[N]`, in a spelling that only raw code can produce.
- The counter is a read-only operand. A helper it is passed to gets its own copy.
- `#[temp]` on an object iterator is an error.
- The compiler rejects a body that contains `raw { .. }`, including raw code in a helper the body inlines.
- The compiler then rejects a body whose final `.air` still writes `temp[N]`. This covers `defer` cleanups and inlined helpers. A write is any of these:
  - an assignment or compound assignment;
  - `inc_lt` / `dec_gt` / `flag_set` / `flag_clear` / `sqrt` on the cell;
  - a `dest: temp[N]` output;
  - a nested `#[temp(N)]` loop.
- Every other use of `temp[N]` in the body must be a plain read: the right side of an assignment or operator, a side of a comparison, or the input of `idle` / `random`. A primitive argument may be an output (`relationship(value: ..)`, `distance_to(.., temp: N)`), so the counter cannot be passed as one. Copy it into a `let` first.
- The check reads the text that is kept, so code a trial lowering discarded cannot hide or fake a step.
- **Limit.** Like `temp[N]` anywhere else, the counter is shared with the trees the body calls. A call that writes `temp[N]` (a `#[temp(N)]` argument or return of a global, or a primitive that writes temps without naming them) changes the loop. That is the point of a temp-backed counter, and it is documented rather than rejected.

**Compound assignment** into target cells already worked and is unchanged.

## Verification

- **`tests/v2_rust_syntax.rs` (12 tests).** Every new spelling is paired with its long form, and each pair must give the same `.air`, the same IFF bytes and the same WAV bank. The error cases are pinned, including the review's regression cases.
- **Full suite.** `cargo test --release -p sims-antics`: 757 passed. `sims-antics test open-pack/tests crates/sims-antics/tests/test_runner`: 30/30, 0 ASSUME.
- **open-pack.**
  - Builds byte-identical before and after its migration to tail expressions.
  - `lint`: 0 findings. The two `state reserved` slots carry an `allow(unused_decl)` that keeps the saved attribute layout.
  - `fmt --check`: clean.
- **Pilots (local only, never committed).**
  - The 16 pilot sources expand to byte-identical `.air` under the old syntax with the old compiler and the migrated syntax with the new one.
  - `lift --v2` over the 13 pilot object files gives 0 different.
  - Its identical / equivalent / inconclusive counts match the pre-branch binary exactly.

## Consequences

- Old-syntax sources no longer parse. Run `scripts/migrate_v2_syntax.py` and then `sims-antics fmt`.
- `std::globals` is the place for further clean-room global signatures. Add one only when its behaviour has been checked against sims-vm.
