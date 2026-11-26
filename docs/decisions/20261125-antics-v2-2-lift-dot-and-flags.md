# `lift --v2` writes named cells, flag methods, unpinned locals and named arguments

*Decided 2026-11-25.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/decompile.rs`; lift test expectations; new
  `v2_2_lift_polish.rs`
- **Track:** Track 9 (`lift --v2`), `.antics` v2.2 Polish Wave, Step 1
- **Builds on:** [`20261125-antics-v2-lift-rust-polish.md`](20261125-antics-v2-lift-rust-polish.md)
  (#476), [`20261125-antics-v2-lint-panic-and-bcon-paths.md`](20261125-antics-v2-lint-panic-and-bcon-paths.md)
  (#478) and [`20261125-antics-v2-typed-std.md`](20261125-antics-v2-typed-std.md)

## Context

The language accepts plainer spellings than the lift wrote:
- the field form of a named motive or skill cell (`sim.motive.hunger`, `sim.skill.cooking`);
- the method form on a `std::flags` field (`self.flags.set(ObjectFlags::InUse)`);
- a local with no `#[slot(N)]` pin;
- an argument named in the signature (`fn f(p0: i16)`).

The lift still wrote these as:
- `sim.motive[Motive::Hunger]` and `sim.person[10]`;
- `self.flags flag_set ObjectFlags::InUse`;
- `#[slot(N)]` on every local;
- `param[k]` for every argument.

## Decision 1: named cells are fields

- **Motives.** `h.motive[Motive::Name]` becomes `h.motive.name`. An index that is a number
  no motive name covers, or a value, keeps its brackets.
- **Skills.** `h.person[N]`, where `N` is one of the six skill words, becomes
  `h.skill.name`. The six are pinned by a unit test against the parser's own table.
- **Personality.** `h.personality.name` was already written before this change, so nothing
  changes here.
- **What qualifies.** `h` must be a path of names, not an indexed cell. Only code tokens
  are rewritten: a `raw { }` body is one token, so `.air` keeps its own syntax.
- **Byte gate.** The field form resolves to a cell at lowering, while the bracket form is
  an index expression. A file that mixes the two could lower differently. So the cells
  are tried together, then in halves, with at most 8 builds per file. A cell that would
  change the bytes keeps its old spelling.

## Decision 2: typed flags are methods

- **Set and clear.** The statement `place flag_set Type::Flag;` becomes
  `place.set(Type::Flag);`, and `flag_clear` becomes `.clear(..)`.
- **Tests.** `place flag_test Type::Flag` becomes `place.contains(Type::Flag)`, and
  `!(place flag_test ..)` becomes `!place.contains(..)`. A test is rewritten only where
  it stands alone as an operand: after `(`, `&&`, `||`, `=`, `,`, a block edge, `if`,
  `while` or `return`, and before `)`, `{`, `&&`, `||`, `;` or `,`.
- **What qualifies.** The flag must be a typed constant: `Type::Name`, with an upper-case
  type that is no BCON table of the file.
- **What keeps the operator.** A bit number (`temp[2] flag_set 3`) keeps `flag_set`,
  `flag_clear` or `flag_test`.
- **Byte gate, one group per place.** The methods need a field that v2 types as
  `bitflags`. Some places are typed as `i16` and refuse every method with a compile
  error: `world.expansions`, `*.wall_placement` and the `attr[N]` cells. So the
  rewrites of one place stand or fall together. The groups are tried together, then in
  halves, with at most 8 builds. The candidates are found again after the cells are
  rewritten, so their byte ranges match the text being changed.

## Decision 3: in behavioral mode, unpinned locals and named arguments

Strict mode keeps both pins: it compares frame storage, and these changes are what move
it.

- **Unpinned locals.** In a function, each `#[slot(N)] let` loses its attribute. A
  function whose `raw { }` code reads `local[` keeps its pins, because that code and the
  declaration share the frame word.
- **Named arguments.** `fn f()` or `fn f(sim: Sim)` under `#[bhav(.. args = N ..)]`
  becomes `fn f(p0: i16, _p1: i16, ..)`, up to the highest argument read, and each
  `param[k]` read becomes `pK`. An argument that is never read is named `_pK`. The
  numbers stay when any of these holds:
  - the function writes an argument: an assignment operator follows it, or `&` or `mut`
    precedes it;
  - the function runs an `animate`, in code or in `raw { }`: its events land in argument
    0 unless a local is named for them, and the build does not restore it;
  - it names an argument other than as `param[k]`, such as a `param:` operand;
  - its `raw { }` code names an argument in any form (`param[k]`, Old Relationship's
    `param:` destination, the `scope(9, k)` cell), or runs a primitive by its bytes
    (`op 0x..`), either of which may write one;
  - the file calls it: each caller would have to pass the arguments;
  - a name `pK` is already taken by a word of the file.

  `sleep` and `idle` count down in argument 0 too, but the build saves and restores it
  around them whenever the tree reads it, so they do not block naming.
- **Checked per tree.** Unpinned locals let the optimizer fold copies, and lint
  `unused_let` exempts only pinned lets. So both byte gates and lint gates apply.
  - Each pass applies every function's change at once, builds once and lints once.
  - A function's change is kept when its own tree builds the same bytes and no new
    diagnostic falls on its lines.
  - The file must then build the identical `.iff` and lint nothing new. Otherwise the
    kept changes are tried together, then in halves, with at most 8 builds per pass.
  - The two passes run one after the other: locals first, then arguments. Run as one
    build, they lost about 40% of the named-argument functions, because a tree that
    failed one kind dropped the other kind too.
  - A text that was linted once is not linted again, so the second pass reuses the
    first pass's last lint.

## Decision 4: the `Table.eN` fallback is gone

#476 wrote `Table.eN` reads that lint `unused_decl` missed. #478 taught the lint to count
`Table::entry` reads, which left the fallback with nothing to do: it wrote 0 such reads in
both modes at the base. The pass and its test are removed.

## Results

The corpus of 1,191 objects and 26,206 trees, lifted in both modes. The base is `main`
at `611795a30`, after #478. Counts are over v2 code, not inside `raw { }`.

| Item | Behavioral before → after | Strict before → after |
|---|---|---|
| `motive[Motive::Name]` | 4,420 → 0 | 536 → 0 |
| `motive.name` | 0 → 4,420 | 0 → 536 |
| `person[N]` on a skill word | 1,398 → 4 | 248 → 1 |
| `skill.name` | 0 → 1,394 | 0 → 247 |
| `personality.name` (already written) | 615 → 615 | 10 → 10 |
| Typed `flag_set` / `flag_clear` / `flag_test` | 6,767 → 2,795 | 453 → 248 |
| `.set(..)` / `.clear(..)` / `.contains(..)` | 0 → 1,769 / 1,507 / 696 | 0 → 45 / 39 / 121 |
| Bit-number flag operators (kept) | 3,392 → 3,392 | 162 → 162 |
| `#[slot(N)] let` | 19,087 → 14,127 (−4,960) | 951 → 951 |
| Functions with named arguments | 0 → 953 (1,150 arguments) | 0 → 0 |
| `param[k]` reads | 11,589 → 9,741 | 961 → 961 |
| `Table.eN` in v2 code | 0 → 0 | 0 → 0 |

- **Typed flag operations left.** The 2,795 behavioral and 248 strict operations that keep
  their operator are on places v2 types as `i16`: `world.expansions`, `*.wall_placement`
  and `attr[N]`. They need `bitflags` types in `std` (Track 8).
- **Skill cells left.** The skill cells (4 behavioral, 1 strict) that keep brackets were turned back by the byte
  gate.
- **Verdicts are unchanged.**
  - Behavioral: 870 identical, 4,651 equivalent, 20,684 inconclusive.
  - Strict: 884, 4,637 and 20,684.
  - There are 0 DIFFERENT, 0 errors or timeouts, and 0 fmt-dirty files.
- **No tree got worse.** No tree lost its proof or its idiomatic form. Inline `raw { }`
  statements are unchanged: 14,553 behavioral, 3,058 strict.
- **Lint is unchanged rule by rule.** Behavioral 263, strict 6, with all style rules at 0.
- **Runtime.** Each pair below ran back to back with 10 workers.

  | Mode | Wall time, base → this change | CPU time |
  |---|---|---|
  | Behavioral | 72.2 s → 88.2 s | 535 s → 629 s (+18%) |
  | Strict | 97.6 s → 90.0 s | 709 s → 693 s (−2%, noise) |

  The behavioral cost is the frame passes: per pass, two builds and two lints of the
  whole file, plus the halving when the per-tree pick fails. The files that pay most are
  large files with many functions, 5 to 6 s more each. Strict mode runs no frame passes.

## Consequences

- Lift test expectations that pinned `Motive::`, typed flag operators or numbered
  arguments in behavioral mode now expect the new forms.
- **Follow-up (Track 8).** `bitflags` types for `world.expansions`, `wall_placement` and
  the `attr[N]` cells would let the rest of the typed flag operations become methods.
- **Follow-up (Track 9).** A tree called in the file keeps numbered arguments. Naming
  them would mean rewriting each call site to pass them.
