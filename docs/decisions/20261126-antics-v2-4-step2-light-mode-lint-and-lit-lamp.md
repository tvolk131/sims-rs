# `.antics` v2.4 Step 2: `light_mode` in the lint, the lamp's light mode, `prefer_if_let` after option aliasing

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:** `crates/sims-antics/src/v2/lint.rs`; tests in
  `tests/integration/{v2_2_step2_lint,open_pack_behavioral}.rs`;
  `open-pack/objects/open-floor-lamp.antics` and its spec; `docs/roadmap.md` (Batch 23). No
  change to lowering, the parser or `v2/decompile.rs`.
- **Track:** Track 7, `.antics` v2.4
- **Builds on:**
  [`20261126-antics-v2-4-light-mode-glob-option-alias-self.md`](20261126-antics-v2-4-light-mode-glob-option-alias-self.md)
  (#495: `light_mode`, option bindings read in place, `self` never bound),
  [`20261126-antics-v2-4-runner-grid-and-if-let-lint.md`](20261126-antics-v2-4-runner-grid-and-if-let-lint.md)
  (#494: `prefer_if_let`)

## Decision 1: `untyped_engine_field` names `data[16]` `light_mode`, through `OBJECT_WORDS` only

`data[16]` joins `OBJECT_WORDS`, beside `light` (`data[51]`). So `self.data[16]`,
`stack_obj.data[16]`, a Sim handle's and a `lot.objects()` handle's `data[16]` are proposed as
`.light_mode`, byte-checked by the trial build like every object word.

It is **not** in `SELF_FIELDS`, although #495's ADR listed both tables. The `SELF_FIELDS` path
proposes its names without asking whether a state of the object already takes the name, and
then `self.light_mode` would be the state, not the word. That table holds the words every
object had a name for from the start (`graphic`, `room_impact`, `id`, ...). Through
`OBJECT_WORDS`, an object with a `light_mode` state keeps `self.data[16]` unnamed, as an object
with a `light` state keeps `data[51]`
(`a_state_named_light_mode_keeps_data_word_16_unnamed`).

- **Which states count.** On `self` and on handles, any state of the file with the word's name
  keeps the cell unnamed, a template's too. That is coarser than needed: an unrelated
  template's state also counts, and so does this object's state for an untyped handle. But it
  has to be. A typed handle (`h: Self`) names its object's states
  (`lower/expr.rs` `handle_has_state`), and in code the build folds away (`if false { ..
  h.data[16] .. }`), the trial build cannot tell the state from the word. Scoping the check to
  each receiver's own states is left for later.
- **`stack_obj`** never names a state in v2, so it now skips the check, and the trial build
  decides. This covers every `OBJECT_WORDS` entry, not only `light_mode`, and changes no
  finding on either lift corpus.
- **The trial budget.** A new word can take trial builds that older findings used. With 48
  trials a file, a file with more `data[16]` statements than that may lose a later finding.
  This is a limit every new word shares; no lifted or open-pack file comes near it.

No `decompile.rs` unit test asks for a lint-clean `data[16]`, so no lift test changes. The lift
does not write `light_mode` yet (it does write `light`), so the behavioral lift now has 48
`light_mode` findings (below), a Track 9 follow-up.

## Decision 2: the Open Floor Lamp sets light mode 1

The lamp's init sets `self.light_mode = 1;` beside `self.flags2.set(ObjectFlags2::LightSource);`,
as the original's light sources do, and never changes it: the switch sets `light` (50 lit, 0
dark), and that alone puts the light out.

- A lit lamp now passes the renderer's `gives_light` (on the lot, the light-source flag, a
  light, light mode 1). So it is a local light for the passes that draw floor pools and light
  on Sims, besides lighting its room (Batch 22).
- **Not visible in a standalone game yet.** Standalone staging ships no `lightMap.dat`, so the
  floor pass has no pool maps. Standalone Sims are drawn by a procedural renderer with fixed
  colours. Toggling the lamp on the starter lot changes the room light only, until both
  passes work standalone.
- `open_pack_behavioral::a_lit_floor_lamp_lights_its_room` checks word 16 = 1 lit and dark, and
  that `gives_light` holds when lit and fails when dark. `sims-antics` does not depend on
  `sims-render`, so the test spells the predicate out beside a pointer to the original.
- `open-floor-lamp.test.toml` expects `16 = 1` after Init and after each switch.
- The lamp's init data changed, so starter-lot saves made by older builds are refused (the
  no-migration rule for open-pack objects).

## Decision 3: the lint's `self`-binding branches go

#495 refuses `self` as a local binding or parameter (`let`, `for`, `if let`, `let .. else`,
`const`, every parameter). Two lint branches only existed for a Sim parameter or an `if let`
binding named `self`, and are removed:

- `untyped_engine_field` skipped `self.data[N]` when a Sim parameter was named `self`;
- the `stack_obj_clobber_after_self` walk did not judge an `if let Some(self)` block.

`self` stays in the lint's list of reserved words, which keeps a suggested name off it.

An import alias can still be named `self` (`use crate::x::GUID as self;`). The constant then
answers to `self` in value position, while `self.field` stays the running object. Neither
removed branch could see it, but the parser accepting the alias is a gap in #495's rule (a
Track 8 follow-up).

## Decision 4: `prefer_if_let` re-measured after #495

Run on the Track 9 v2.4 lifts (1,191 files each). Base is `88e798d3b`'s lint; "proposed" counts
the rewrites the rule hands to the trial build (a probe build, not committed).

| Corpus | Proposed | Kept, #494 on v2.3 | Kept now |
|---|---|---|---|
| `lift-v2-4-beh` | 154 | 16 / 148 | 21 (14 `let .. else`, 7 `if let`) |
| `lift-v2-4-strict` | 26 | 0 / 26 | 0 |

- The kept findings are 19 `find_companion` and, new with #495's in-place option bindings, 2
  `take_elapsed_periods` whose value goes into a compound store
  (`me.person[28] -= periods;`), which #494 could not keep.
- All 21 were applied together per file (8 files). Each rewritten pack builds a FAR
  byte-identical to the original, passes `fmt --check`, and has no `prefer_if_let` left: no
  false positives and no broken rewrites.
- **Why the other 133 behavioral proposals still drop:** 71 are in functions with a raw block
  or a loop, which #495's alias declines (raw text may name frame words; a loop may write the
  temp). The other 62 have the kept shape, yet the alias does not apply. Probes of one tree text show it depends on the tree, not the text:
  `let Some(periods) = take_elapsed_periods(20) else { return true; };
  sim.person[28] -= periods;` builds the `temp[0]` form's bytes with a `sim: Sim` parameter or
  a fixed `args = 0, locals = 0` header, and other bytes on `me` with no header, on `stack_obj`,
  and as `if let` on `me`. That is a Track 8 follow-up; the lint stays as it is.
- The strict corpus's 26 proposals all still build other bytes.
- Other findings: the only change is `untyped_engine_field` on `self.data[16]`. There are 48
  findings in the behavioral corpus (every `data[16]` outside raw blocks, in 38 files, each
  `self.data[16] = 1;`), and 0 in the strict one, whose 53 `data[16]` sites are all in raw
  `.air`. All 48 applied together build
  byte-identical FARs, pass `fmt --check`, and leave no `data[16]` finding.

## Consequences

- An open-pack object can now be a full light source without raw `data[N]`.
- `prefer_if_let`'s recall now follows lowering: once option bindings read in place in more
  trees, more proposals survive, with no lint change.
- Track 9 can write `light_mode` in the lift, as it writes `light`.
