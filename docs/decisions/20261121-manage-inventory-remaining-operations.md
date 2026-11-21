# Manage Inventory: operations 1, 2, 5, 6 and 7+

*Decided 2026-11-21.*

Manage Inventory (primitive 51, `0x4552f8`) previously supported only
operations 0 (add), 3 (find first) and 4 (walk) and failed loudly on the
rest. This completes the remaining operations in `crates/sims-vm/src/primitives/inventory.rs`
directly from the original's dispatch (`0x4553ba` to `0x455749`):

- **Common setup (`0x455304`–`0x4553b5`).** Every operation resolves the
  owner (caller or temp 4 with flag `0x2000`; native error `0x1c` if not a
  person), looks up its neighbor record (`0x42571f`), gets or creates its
  inventory (`0x42a8a1`), and substitutes the stack object's GUID when the
  operand GUID is 0 (`0x455397`) before switching on the operation byte.
- **Operation 1 (`0x45569d` → `0x41f427`).** Reads `count` (1, or caller's
  temp `(flags >> 2) & 3` with flag `0x2`). A count of 0 returns true
  without searching. Otherwise it finds the first token of `(kind, guid)`
  (`0x41f721`, returning false if none), always subtracts `1` (`0x41f447`
  pushes `1` into `0x41f471`, clamping signed counts `<= 1` to `0`), and
  erases the token (`0x59b11c`) if the resulting count is `0` or the
  requested count was `-1`, returning true.
- **Operation 2 (`0x45560c` → `0x41f56b`).** Reads `count` (same as
  operation 1) and `index` from the caller's temp `(flags >> 8) & 3`. A
  count of 0 returns true without checking `index`. A negative or
  out-of-range `index` returns false. Otherwise a negative `count` erases
  the token at `index`; a positive `count` subtracts from its signed count
  and erases it when the result is `0` or less, returning true.
- **Operations 5 (`0x455437`) and 6 (`0x4553e8`).** Look up the neighbor
  record at the 1-based slot in the caller's temp 0 (`0x4256f4`) and read
  its GUID (`0x5ed65e`, record +4). If no record exists at that slot, the
  port fails loudly (`Trap::Unsupported`) rather than dereferencing null
  (`0x45544b` / `0x4553fc`). Otherwise they remove every token of kind `1`
  (op 5) or kind `3` (op 6) (`0x41f4e5`), add one token of that kind and
  GUID via `0x41f38d` (no-op if the inventory already holds `0x7fff`
  tokens), ignore `0x41f38d`'s return value, and take the true branch
  (`0x455418`).
- **Operations 7 and above (`0x4553e6` → `0x455418`).** Take the true
  branch after the common setup.

**Unverified:** no emulator recording covers Manage Inventory operations
1, 2, 5, 6 or 7+ yet; the behavior is checked against the disassembly and
synthetic unit tests. The interface inventory dirty flag (`0x4ceceb` with
`0x4000000` at `0x455418`) remains unmodeled presentation.
