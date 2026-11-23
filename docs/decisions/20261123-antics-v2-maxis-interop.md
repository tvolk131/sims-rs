# `.antics` v2 Maxis Interop & Zero-Raw ABI Sugar: `extern mod`, Sparse States, Named BCON Tables, Standalone Trees

**Date:** 2026-11-23  
**Branch:** `antics/v2-maxis-interop-abi-sugar`  
**Track:** Track 8 (`.antics` v2 Language Ergonomics & Extensions — PR C)

## Context

After PR A (#395) and PR B (Tier 3), v2 objects still had to fall back to `raw { ... }` blocks or invented layouts to talk to code they don't own:

1. **Maxis global/semi-global BHAVs.** You could only reach global (256..=4095) or semi-global (8192..) trees from raw `.air` `call N(...)` lines. Arguments were hand-placed in `temp[N]`, and true/false/error exits were wired by hand.
2. **Fixed attribute layouts.** State slots were always `0..N` in declaration order. You could not match an existing object's attribute map (e.g. "`attr[7]` is the lamp's glow"), and every state was zeroed in `init`.
3. **Private constant tables beyond BCON 4096.** Only i16 `param`s (auto BCON 4096) were expressible. There was no way to declare BCON 4097+ or reference Maxis global/semi-global constants.
4. **Shared, non-inlined code.** Helper `fn`s always inline. SimAntics check trees (`CT - Is Fridge`) and value trees that other code `call`s needed a real BHAV with a pinned ID.

Every feature lowers onto the existing `.air` syntax. Nothing changes in `.air`, the verifier, `sims-vm`, `sims-ui`, `sims-game`, or `open-pack/`.

## Design & Lowering Summary

### 1. `extern mod`: Typed Global/Semi-Global BHAV Calls

```antics
extern mod light_globals {
    @id(256) fn add(a: i16, b: i16) -> i16 @ temp[0];
    @id(257) fn is_dark(level: i16 @ temp[4]) -> bool;
    @id(258) fn try_light(power: i16) -> Result;
}
extern mod lamp_semi { @id(8192) fn doubled(x: i16) -> Result<i16 @ temp[1]>; }
```

- **Where it can appear:** at module level (`pub` is importable with `use`) or as an object/template member. Calls are written `light_globals::add(...)`.
- **Valid IDs:** `@id` must be global (`256..=4095`) or semi-global (`8192..`). Private IDs are rejected.
- **Parameters:**
  - Positional parameters become the callee's `param[i]`.
  - `name: T @ temp[N]` binds the argument into the shared thread temp `temp[N]` (`0..=7`).
  - Duplicate temp bindings, `temp[N >= 8]`, default values, and more than 8 positional parameters are compile errors.
  - In a signature that mixes both kinds, a temp binding below the positional count is an error: `call(temps)` would overwrite it.
- **Call encoding:**
  - `call ID(a, b)` (literal operands) is used when every positional argument is a compile-time constant. Constants include literals, bools, i16 params, enum constants and private BCON entries. There must be at most 4, and not all four may be `-1` (the `call(temps)` sentinel).
  - Otherwise the call uses `temp[i] := arg_i; ...; call ID(temps)`.
  - Temp-bound arguments are always written as `temp[N] := expr;` before the call.
  - Arguments are evaluated left to right before any temp is written. An operand that reads a temp, or one followed by an argument that may run code, is snapshotted into a fresh local first.
- **Return ABIs:**

  | Declared | Statement `f();` | `f()?;` | Condition `if f()` | Value `x = f()` |
  |---|---|---|---|---|
  | *(none)* | both exits continue | error | error | error |
  | `-> bool` | both exits continue | `if !call { return error; }` | `call ...` | `0`/`1` local |
  | `-> Result` | **error: unused Result** | `if !call { return error; }` | `call ...` | error |
  | `-> T @ temp[N]` | both exits continue | error | error | `temp[N]` |
  | `-> Result<T @ temp[N]>` | **error: unused Result** | `if !call { return error; }` | error | `f()?` gives `temp[N]` |

  "Both exits continue" is lowered as `if call ... { }`. With `let x = f()` and `x = f()` into a plain target, `temp[N]` is read directly with no extra local. In any other expression position the result is copied into a fresh local right after the call.
- **`return error;`** is a new statement that exits through the BHAV error branch after running every active cleanup (all scopes, including an inlined helper's caller).
  - Postfix `?` on a call lowers as described above.
  - `cond?;` on any bool expression is sugar for `if !cond { return error; }`.
  - `return error;` and `?` are rejected inside `defer`.
- **Why `if call X { }` rather than a bare `call X;`:** `.air` routes a bare call's false exit to `return false`. The empty `if` sends both exits to the next node: one node, no extra instructions.

### 2. Sparse / Explicit-Slot States

```antics
state glow @ 9: i16;          // binds my.attr[9]; init does NOT touch it
state quality: i16 = 3;       // auto slot; unchanged behaviour
```

- **Shared allocator.** `lower_object` and imported object schemas (`build_object_schema`) use the same allocator, so typed cross-object access sees the same slots. Explicit slots are reserved first. Auto states then take the lowest free slot in declaration order. Without explicit slots the layout is exactly the old `0..N`, which keeps open-pack byte-identical.
- **Validation.** Overlapping explicit slots, and any slot above 255, are compile errors.
- **Init behaviour.** An explicit-slot state *without* an initializer is not written by `init`, so it keeps whatever the VM or a previous owner left there. An explicit slot *with* an initializer, and every auto state, are initialized as before.
- **Attribute count and map.** `attributes:` auto-sizes to `max(max_slot + 1, explicit attributes or 8)`. The `// Attributes:` comment is sorted by slot.

### 3. Named BCON Tables

```antics
bcon 4097 "LampTuning" { cheap = 10, bright = 25 }     // object/template member
extern bcon light_consts { table_height_flag @ 16521; by_chunk @ 257[3]; }
```

- **Private tables.** Allowed IDs are `4096..=4159`. 4096 is allowed only when the object has no i16 `param`s. A table holds at most 128 entries, with unique entry names and unique table IDs.
  - Tables are emitted as `bcon ID "Label" = [v, ...];`.
  - Operands are numeric `bcon[(table << 7) | entry]`, e.g. `LampTuning.bright` gives `bcon[129]`. This keeps v2 tables out of `.air`'s single global BCON-name namespace.
  - Tables are referenced as `LampTuning.x` or `lamp_tuning.x`. Their values are known at compile time, so they fold into literal call operands and constant contexts (e.g. a non-zero divisor).
- **`extern bcon`.** Accepts a raw operand address (`@ 16521`, table ≤ 191) or `@ chunk_id[entry]`. It lowers to `bcon[addr]` and emits no chunk. The value is only known at runtime, so it is rejected in constant-only positions.
- **Address math.** The address math matches `sims-vm` `Program::constant`:
  - tables 0..63 map to private `4096 + t`;
  - tables 64..127 map to semi-global `8192 + t - 64`;
  - tables 128..191 map to global `256 + t - 128`.
- **Read-only.** Assigning to a BCON entry is a compile error.
- **Name priority.** A BCON table name takes priority over a same-named variable when used as `Name.entry`, consistently in lowering and in `stack_obj`-clobber analysis.

### 4. Standalone `tree` Declarations

```antics
@id(4143) tree "CT - Is Fridge" { return true; }                      // ident: ct_is_fridge
tree score "Quality Score" -> i16 @ temp[0] {
    if self.quality > 0 { return (self.quality * 24) / 5 + 1; } else { return 0; }
}
tree bump "Bump Hits" (sim: Sim, n: i16) { self.fridge_hits += n; }  // Sim-on-object context
```

- **Declaration.** A `tree` is a real BHAV inside an object or `pub template`. The optional ident is derived from the label in snake_case when omitted.
  - An optional `@id` must be a private ID and is checked with the same collision rules as other pinned trees.
  - Optional parameters are positional (`param[i]`, header `(args: N)`) or `@ temp[N]`. Temp-bound parameters are snapshotted into locals on entry, because temps are shared with callees.
  - The optional return ABI is the same as for `extern mod`.
- **Calls.** Sibling code calls a tree by ident through a real `.air` `call`; trees are never inlined. IDs are allocated before any body is lowered, so trees can call each other (and themselves).
- **Context.**
  - A tree whose first parameter is `name: Sim` runs in the Sim-on-object context: `self.x` is `stack.attr[i]`. Callers must be in that context and must pass the Sim handle first.
  - Otherwise the tree runs in the object's own context (`my.attr[i]`) and is callable only from `init`/`main`/`load`/`reset`/`cleanup`/object trees.
  - A mismatch is a compile error. BHAV calls copy, but never propagate back, the caller's `stack_obj`, so calls never clobber it.
- **No `->` means a check tree.** A tree without `->` behaves like a SimAntics check/action tree: it may `return true/false/<bool>`, falls through to `return true`, and can be used as a condition.
- **Must return on every path.** Trees declaring `-> bool`, `-> T @ temp[N]` or `-> Result<T @ temp[N]>` must return on every path. Value returns write `temp[N]`. When the expression is alias- and trap-safe and no cleanups are active, compound arithmetic lowers in place into `temp[N]`. The example above is 6 nodes in total: compare, 4 in-place arithmetic, and `else`.
- **Reserved idents.** Tree idents may not shadow built-in calls or helper `fn`s.

### 5. `destroy_self(flags: N | no_yield: b | skip_resets: b)`
Lowers to `remove_object(me|stack_obj, flags: N)`, using the same bits as `.air` (`no_yield` = 1, `skip_resets` = 2). Plain `destroy_self()` is unchanged.

## Verification

- **New synthetic tests** in `crates/sims-antics/tests/v2_compiler.rs`. There are no commercial assets. The global and semi-global libraries are `.air` stand-ins compiled into the fixture.
  - Exact `.air` and bytecode shapes:
    - literal operands `[2,0,10,0,..]`;
    - `call(temps)` operands `0xFF`×8;
    - `?` exits with `on_false = 0xFD`;
    - ignored results have `on_true == on_false`;
    - a 6-node in-place value tree;
    - the BCON 4097 chunk `[10, 25]`, with no chunk emitted for `extern bcon`;
    - sparse `my.attr[9]` with no init zeroing and `attributes = 10`;
    - hole-filling auto slots;
    - mixed positional + temp-bound tree calls;
    - `remove_object(me, flags: 3)`.
  - Live `sims-vm` runs:
    - global `add` through literal and `call(temps)`;
    - `temp[4]`-bound `is_dark`;
    - `bcon[16521]` reading global BCON 257 entry 9;
    - a semi-global `Result<i16 @ temp[1]>` with `?`;
    - a value tree, a CT marker used as a condition, and a Sim-context tree called from an interaction;
    - `?` failure trapping `ErrorBranch` before any state write, plus the success path.
  - 15 negative diagnostics, all with spans:
    - duplicate temp binding, temp ≥ 8, colliding slots;
    - invalid BCON table ID, invalid extern BHAV ID;
    - a positional/temp collision;
    - unused Result, `?` on a value ABI, writing to BCON;
    - wrong tree context, a value tree missing a return, `?` in `defer`;
    - a BCON address out of range, a tree shadowing a builtin, BCON 4096 conflicting with params.
- **Parity.** All 20 `open-pack/objects/*.antics` expand to **byte-identical** `.air` against `main` (`ce1110c4`, CLI `expand --pack open-pack`). `sims-game --test cheats_cache_and_open_pack` passes 7/7.
