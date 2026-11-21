# Visitor category 4 and 7 actions check the hotel-key token

*Decided 2026-11-21.*

## What the original does

In `0x458319` (the action check shared by autonomy's `autonomy_allows`,
pointer `highlight` and `check_chosen`), after target word 8 bit 9 (and
target word 25 > 0 in autonomy/highlight mode), when an interaction table
entry lacks flag 1 (`VISITORS`), the caller is a visitor (person data word
32 nonzero), and the lot is either residential (`0x40745f == 1`) or the
entry lacks flag 0x20 (`COMMUNITY`):

1. `allowed` is cleared to false (`0x4583ed`).
2. If the target's category (object data word 59, `+0xc0`) is `4` or `7`
   (`0x4583f1`–`0x4583fb`):
   - It reads the caller's neighbor ID from person data word 31 (`+0x5ca`,
     `0x458403`) and looks up that neighbor's inventory in the neighborhood
     via `0x42a8a1` (`0x45840e`). If the neighbor has no inventory yet,
     `0x42a8a1` allocates an empty one and appends it to the neighborhood's
     inventory vector.
   - It asks `0x43e21a` (`0x458427`) on the object registry for the GUID of
     the hotel-key token: on first call while its static cache (`0x6e4f5c`)
     is 0, `0x43e21a` walks the registry's definition vector (`+0x20`,
     `0x43ac3a`) and writes the GUID (`OBJD +0x1c`) of each definition with
     object type `0xd` (`OBJD +0x12`) into `0x6e4f5c`, so the last
     registered type-`0xd` definition wins (or `0` when none exists). In
     Complete Collection there is one such definition: `Token - Hotel Key`
     (`0x3ac022d6`, `ExpansionPack4/ExpansionPack4.far::Tokens.iff`, OBJD
     #16810).
   - It calls `0x41f660(0, guid, &index)` (`0x458432`), which finds the
     first item in the inventory with `kind == 0` and `guid == guid`
     (`0x41f721`) and returns its 16-bit count in `AX` (or `0` if absent).
   - If the returned count is strictly positive as a signed 16-bit integer
     (`TEST AX, AX; JLE`, `0x458437`–`0x45843c`), `allowed` is set back to
     true.
3. The age/kind check (`0x458443`) then runs as usual, so even when the
   age/kind check rejects the entry, `0x42a8a1` has already created the
   empty inventory if it was absent.

Earlier notes had guessed `0x41f660` was a relationship query; reading
`0x458319`, `0x42a8a1`, `0x43e21a` and `0x41f660` in the binary shows it is
the Vacation hotel-key inventory token lookup.

## What changes here

- `Machine::inventory_slot` (`0x42a8a1`) and `Machine::find_inventory_token`
  (`0x41f660`, `0x41f721`) are shared helpers in
  `crates/sims-vm/src/primitives/inventory.rs`, reused by both Manage
  Inventory operation 3 (testing `count != 0`) and `0x458319` (testing
  `(count as i16) > 0`).
- `Machine::hotel_key_guid` (`0x43e21a`) scans the reloaded registry order
  (`Characters` definitions in `Program::characters` after the rest, as
  `0x43c228` reregisters them) in reverse for the last definition with
  `kind == 0xd`, returning `0` when none exists. Dynamically created
  characters are people (type 2).
- `Machine::visitor_has_hotel_key` reads `caller`'s person data word 31,
  calls `find_inventory_token(neighbor, 0, guid)`, and tests
  `(count as i16) > 0`.
- Both `autonomy_allows` and `check_chosen` in
  `crates/sims-vm/src/autonomy.rs` read the caller's live visitor, age and
  kind words on every check (as `0x458319` does) and call
  `visitor_has_hotel_key` on category 4 or 7 targets instead of returning
  `Trap::Unsupported`.

## Not verified

- No emulator recording exercises a visitor holding `Token - Hotel Key` on a
  category 4 or 7 object; that branch is verified from the disassembly and
  synthetic fixture tests only.
- In the original executable, `0x43e21a`'s static cache (`0x6e4f5c`) keeps a
  nonzero GUID for the lifetime of the process across later neighborhood
  reloads; here each `Lot` derives the GUID from the current neighborhood's
  reloaded registry order. Complete Collection has one matching definition
  outside `Characters`.
- `0x458826` (`autonomy_entries`) has a separate hotel-key check gated on
  Vacation lots 40 to 49 (`0x4074c8`), which `Find Best Interaction` still
  refuses earlier as unported.
