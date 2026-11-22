# Destination-lot primitives, autonomy, and Superstar scope 38

*Decided 2026-11-22.*

## Context

Booting and stepping destination lots across all five expansion travel zones
(Downtown `House21`, Vacation Island `House40`, Old Town community `House58`,
Studio Town `House81`, and Magic Town `House90`/`House93`) reached unported
paths in `Sims.exe` 1.91:

1. **`Find Best Interaction` (`0x45dd39`)** previously trapped on non-residential
   lots, on `global[9] == 0`, and on free-will-off visitors, and lacked the
   Vacation Island hotel-key branch in `autonomy_entries` (`0x458826`).
2. **`Generic Sims Call` routines `18` (`0x4506fb`) and `26` (`0x450732`)**
   were unported (hit on Downtown `House21` and Magic Town lots).
3. **Memory operand scope `38` (`0x454304`)** ("neighbor's definition") was
   unported (hit on Superstar Studio Town `House81`).
4. **Live stepping on `House81` and `House40`** also hit three small unported
   VM paths:
   - Door-edge wall obstacles in the route planner (`0x482625..0x482758`,
     decrypted from `dump-0725/00482000.bin`).
   - Graphic-paired door partner lookup (`0x4636c5`).
   - Container-slot horizontal animator offset on `Snap` (`0x454ceb..0x454d5e`
     → `0x58933b` → `0x5921f0`).

## What the original does

### 1. `Find Best Interaction` (`0x45dd39`) and `autonomy_entries` (`0x458826`)

- **Free-will-off visitors (`0x45dd87..0x45ddaa`)**: A visitor (`person.data[32] != 0`)
  is only refused autonomy when `person.data[61]` matches the controlled
  family's ID (`game +0x10 -> +0x10c`) and `person.data[65] <= 1`.
- **Controlled family ID 0 (`0x45ddb0..0x45ddd5`)**: Always queries
  `0x4e97dc(0)` → `0x4e9608`; if the controlled family's ID is 0 and that
  person is `me`, returns false. With `active_family: Some(&[])` (unoccupied
  lots), `0x4e97dc(0)` returns `-1` and autonomy proceeds; bare callers with
  `active_family: None` still fail loudly.
- **Unzoned 4-phase scan (`0x45de58..0x45de72`, `0x45e024..0x45e066`)**: When
  `zoning == 0` (`0x45e2be == 0`), `quarter = ([SIMI + 0x98] + me) & 3` where
  `[SIMI + 0x98]` is `World::ticks`. Phase `0` scans all objects; phase `1`
  skips objects with `distance > 7.0f` (`0x666a50 = 0x40e0_0000`, `FCOMP` +
  `JA`); phase `2` keeps even IDs; phase `3` keeps odd IDs.
- **Community / Vacation master filter (`0x45e2db`, `0x45df43..0x45df8e`)**: On
  community (`zoning == 2`) and Vacation Island (`40..50`) lots, objects whose
  master definition (`0x45728d`) has word 69 (`+0x8a`) equal to `4` or `5` are
  skipped.
- **Self-skip (`0x45dfae`)**: `id == me` is skipped unless `person.data[65] > 1`.
- **Vacation Island hotel filter (`0x4074c8`, `0x45de77..0x45ded8`,
  `0x45dfca..0x45e024`)**: On lots `40..50`, when `person.data[61] != global[9]`
  and any object on the lot has `data[59] == 0x2e` (46), objects in an indoor
  room (`room != -5 && !room.outdoors`) with `data[59]` in `5 | 6 | 7` are
  skipped.
- **Vacation Island hotel-key entry gate (`0x45891b..0x4589e4`)**: Inside
  `autonomy_entries`, on lots `40..50` for targets with `data[59]` in `4 | 7`,
  the caller's inventory (word 31, created empty via `0x42a8a1` if absent) is
  queried for the hotel-key token (`0x43e21a` + `0x41f660`, testing `count != 0`
  via `test ax, ax; je`). A zero count skips the entry; a nonzero count checks
  only the dog/cat flags and emits the entry immediately.

### 2. `Generic Sims Call` routines `18` (`0x4506fb`) and `26` (`0x450732`)

Both query `0x4e97dc(0)` (member 0 of `game +0x10`, verified against portrait
table `0x4e9753`). When `0x4e97dc(0)` returns `-1`, both leave temps untouched
and return true.

- **Routine `18`**: Writes the first on-lot person matching member 0
  (`0x4e9608`), or `-1` if absent, into `temps[0]`.
- **Routine `26`**: When the controlled family's ID (`+0x10c`) is nonzero,
  writes `(family_id, 1)` into `(temps[0], temps[1])`. When it is `0`, writes
  `(first_on_lot_person, 0)` (or fails with `Trap::Unsupported` if absent,
  where `0x450764` dereferences null).

### 3. Operand scope `38` (`0x454304`, "neighbor's definition")

Looks up the 1-based neighbor record from `stack_object` (`0x4256f4`, throwing
native error `0x37` if missing), checks `0 <= index < 108` (`DEFINITION_WORDS`,
throwing native error `0x23` if out of bounds), and reads word `index` of that
neighbor's linked object definition (`[neighbor + 8] -> [+0x60] + 2*index`) as
a read-only `Cell::Literal` (tail `0x453b51`, no word pointer).

### 4. Destination-lot routing and `Snap` support

- **Door-edge obstacles (`0x482625..0x482758`, `crates/sims-vm/src/routing/planner.rs`)**:
  For `DOOR` (kind 8) objects with rotated wall mask bits in `0x55`, each set
  bit in `0x01`, `0x04`, `0x10`, `0x40` replaces the single obstacle rectangle
  with a 1-unit footprint edge strip at cost `10_000` and calls
  `InflateRect(3, 3)`, before the common `IntersectRect` against the actor
  (`0x48276f`) and second `InflateRect(3, 3)` (`0x48279f`).
- **Graphic-paired doors (`0x4636b5..0x463750`, `crates/sims-vm/src/routing/rooms.rs`, `portals.rs`)**:
  `partner()` reads definition word 69 once from the starting `door` (`EDI` at
  `0x4636b5`) and keeps that pairing mode across retries; when word 69 is `1`,
  it scans the group list for the first other tile matching `current`'s
  definition word 8 (`+0x10`). Returning to `door` or finding no match returns
  `Ok(None)` (`0x463750` returning null), which `portal_distance` (`0x463adf`)
  handles by skipping the cross-room edge while still exploring same-room
  portals; callers that dereference the pointer (`0x4822cb`, `choose_portal`,
  `try_portal`, `fail_portal`) use `require_partner` and fail loudly on `None`.
- **Container-slot animator offset (`0x454ceb..0x454d5e` → `0x58933b` →
  `0x5921f0`, `crates/sims-vm/src/primitives/snap.rs`, `head.rs`, `memory.rs`, `placement.rs`)**:
  `Snap` into a container slot rotates `entry.offset[0..2]` by
  `container.facing & 7` (even facings only; odd facings yield `(0, 0)`),
  truncates with `__ftol`, and sets `animator+0x8c..+0x94` for persons.
  `cTSPerson::Place` (`0x5921f0`) leaves `a.position` unshifted and adds
  `+0x8c..+0x94` when computing terrain altitude and the model matrix; `head.rs`
  derives this offset from `(object.data[2], object.data[3])` while the person
  is contained so `sims-sim` and `boot/save.rs` stay unchanged. To prevent this
  derivation from silently diverging from the stored `0x58933b` offset, the VM
  guards the divergent transitions with `Trap::Unsupported`: rotating a
  container when the facing change shifts a held person's computed offset
  (`memory.rs::store`, `primitives/snap.rs`, `placement.rs::turn_one`, and
  `placement.rs::turn_group`), writing a person's `data[2]`/`data[3]` directly
  (`memory.rs::store`), placing a person into or moving a person out of a
  nonzero-offset container slot outside `Snap` (`placement.rs::put_one`), and
  taking a person off the lot from such a slot (`placement.rs::take_self_off`
  and `placement.rs::take_off_to_remove`, before `0x4332ed` refreshes the
  animator).
- **Family 0 active roster (`0x415fcf..0x415fd7`, `crates/sims-vm/src/boot.rs`)**:
  When `self.load.house != 0 && self.load.family == 0`, `boot.rs` passes the
  actual members of family `0` if present in `world.families` (else `&[]`) as
  `active_family`.
- **Scope 38 duplicate-GUID guard (`0x45434b`, `crates/sims-vm/src/memory.rs`)**:
  Because `sims_sim::neighbors::Neighbor` stores only `guid` (not the linked
  definition pointer), scope `38` fails loudly with `Trap::Unsupported` if more
  than one catalog definition shares `record.guid`, matching `selection.rs`.

## Vacation `House40` note

In `Sims.exe`, Vacation Island lots only enter Live Mode with a traveling
household (`global[3] != 0`). Stepping `House40` with `global[3] == 0` hits a
script bug in `NPC_Vacation_Controller.iff` BHAV `0x1006` (`Control Vacation
Director`): node 44 skips node 20 (`local[6] := selected.person_data[61]`) when
`global[3] == 0`, so node 22 reads uninitialized `local 6`, which overlaps half
of a popped frame header's `cXBHAV*` pointer in `frame_storage`. Selecting a
Sim on `House40` exercises the live Vacation controller cleanly for 300 ticks
without relaxing `check_local`.

## Not verified

- None of the non-residential destination-lot branches (`autonomy.rs`
  community/vacation/unzoned/hotel-key rules, `Generic Sims Call` `18`/`26`,
  scope `38`, door-edge obstacles `0x482625`, graphic-paired doors `0x4636c5`,
  and container-slot animator offsets `0x454ceb`) have been compared against a
  Windows emulator recording; all are verified from `Sims.exe` 1.91 disassembly,
  synthetic unit tests, and live Complete Collection multi-destination stepping.
- Deriving the container-slot animator offset from `(object.data[2],
  object.data[3])` in `head::model` is guarded against container facing writes
  and turns (`memory.rs::store`, `primitives/snap.rs`, `placement.rs::turn_one`,
  `placement.rs::turn_group`), direct container/slot writes, non-`Snap`
  container entry/exit (`placement.rs::put_one`), and person take-off from a
  nonzero-offset slot (`placement.rs::take_self_off`,
  `placement.rs::take_off_to_remove`) via `Trap::Unsupported`, so divergent
  cases fail loudly instead of silently drifting.
- Residential lot determinism is verified against the existing 43,200-tick Goth
  day hash (`df6eb544be87b93233011f355999ac9abb0c36dc5e914f2f7c0fe2f49967b19a`)
  and 8/8 ratchet suite; note that previously unported error paths (such as
  free-will-off visitors on residential lot 0) now execute the ported
  `0x45dd87..0x45ddaa` logic instead of returning `Trap::Unsupported`.
