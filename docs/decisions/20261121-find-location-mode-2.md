# Find Location For mode 2 (`0x44b484`)

## Context

`Find Location For` (opcode 16, `0x44b3c8`) had modes 0, 1, 3, 4, 5, and `>= 6` ported; mode 2 ("in front of" / at the reference object facing opposite it) was the last remaining mode trapping as unported.

## Behavior recovered from `Sims.exe`

Disassembly of `0x44b484..0x44b503` and `0x44b70d..0x44b721`:

1. Target (`esi`) and optional local reference (`edi` when `flags & 1 != 0`, errors `0x15`, `0x33`, `0x17`) are already resolved in the shared prologue (`0x44b3c8..0x44b482`).
2. If `flags & 1 == 0`, `edi` is null and defaults to `self.me` (`0x44b48f..0x44b493`).
3. The reference object's `[x, y]` (`[edi + 0xf4]`, `[edi + 0xf8]`) is captured before turning (`0x44b496..0x44b4a7`).
4. Turn step count (`0x44b4ab..0x44b4cf`):
   `delta = (((ref_facing + 5) % 8) - target_facing + 8) % 8`, `steps = delta / 2` (signed truncation toward zero, matching x86 `cdq; idiv 8` and `cdq; sub eax, edx; sar eax, 1`).
5. Target rotates via virtual function `+0x4c` (`0x44b4d2`): `turn_group` (`0x422937`) for multi-tile groups and `turn_one` (`0x433f75`) for one-tile objects and persons.
6. The reference object's level (`[edi + 0x110]`) is read after turning (`0x44b4d5`) and tested with `Fits` (`vtable + 0x54`, `0x44b4e8`). If `Fits` returns false, mode 2 returns false (`0x44b4ed` -> `0x44b71b`) without undoing the rotation.
7. Otherwise, the reference object's level is read again (`0x44b4f9`) and `Put` (`vtable + 0x58`, `0x44b70d`) places the target with `notify = true`. Mode 2 jumps directly to `0x44b562` (returning true), skipping `search`, `flags & 2`, `flags & 4`, RNG draws, and the graphics refresh (`vtable + 0x74`, `0x433bb3`).

## Verification

- Covered by unit tests in `crates/sims-vm/tests/interpreter/find_location.rs` (all 8 reference facings × 4 target facings, `flags & 1` local reference and ignoring `flags & 2`/`4` and `GRAPHICS & 1`, keeping rotation when `Fits` fails, error precedence `0x15`/`0x33`/`0x17`, multi-tile group rotation including `steps == 0` off-lot behavior, and re-reading reference level after `Rotate` callbacks).
- **Unverified against a Windows recording**: none of the current Goth or expansion recordings execute Find Location For mode 2 yet.
