# Avatar search paths and Animation.ndx for People::load and skeleton lookup

*Decided 2026-11-21.*

Follow-up to PR #293 (`20261120-neighborhood-open-replays-five-pools.md`,
Sol's P2 on avatar search paths) and Sol's P3 on UTF-8 BOM stripping in
`sims_formats::strings::decode`.

## Avatar registry (`sims_content::AvatarRegistry`)

Previously, `People::load` and `skeleton_scales` scanned `GameData/Animation`'s
`.far` archives and every `.bcf` in the indexed object directories (`Index`).
That missed eager `GameData/Skins` overrides (including `.cmx` catalogs),
bypassed `Animation.ndx`, and let undecodable `.bcf` files outside the
original's avatar search paths block skeleton or suit lookups.

`AvatarRegistry::load` now shares `import_wardrobe::mount_avatar_paths`
(`0x43ae81`, `0x43bdc5`, `0x414633`, `0x5835ed`) with lenient archive
mounting (`0x5ba8d6`: an unreadable FAR such as a malformed archive in
`GameData/Objects` is skipped instead of aborting avatar loading) and replays
the original's two-pass startup registration:

1. **Eager pass (`0x59f844`).** Walks the `0x5835ed` `appearance` paths
   (`GameData/Skins` first, `GameData/cmx*`, `ExpansionShared/*` including
   `ExpansionShared/Skeletons`, installed expansions in reverse, and loose
   download providers in reverse), reading all `.cmx` catalogs (`0x59e746`,
   `0x594d73`) before all `.bcf` catalogs (`0x59f087`, `0x595140`).
   - Skeleton names keep their exact byte case (`0x59f163`) and the first
     registration wins (`0x59f132`, `0x59f18c`).
   - A `.cmx` file with a non-zero skeleton count (text skeletons are not
     implemented), a `.cmx`/`.bcf` missing its skeleton count, or a
     `.cmx`/`.bcf` file whose skeleton group fails to decode records an
     ordered blocking error, so any later-registered or missing skeleton
     fails loudly naming the file instead of silently falling back.
     Earlier-registered skeletons stay valid.
   - Appearances are read record-by-record into `suits`, lowercased, with
     per-file decode errors recorded in `suit_errors`.
2. **Indexed pass (`0x584562` → `0x5a0212` → `0x59fc48`).** Walks the
   `indexed` paths (`GameData/Animation`, plus download providers that
   hold `<stem>.ndx`) and streams each `.ndx` file with `IndexReader`.
   - Each `Kind::Skeleton` entry whose exact name is not already registered
     (`0x59fd07`) stores a lazy reference `(directory, catalog + ".bcf")`
     (`0x59fd18`).
   - On lookup (`0x5a0384`, `0x5a280f`), an indexed entry opens
     `directory + catalog.bcf` through `MountedFiles::lookup` (`0x5a3810` →
     `0x59e60e`) and reads its `Kind::Skeleton` group via `Binary`. Every
     skeleton decoded before any error is immediately registered in the
     registry (`0x59f087`, filling existing unloaded entries via `0x5a2207`
     and keeping already-loaded ones via `0x59f18c`), so sibling skeletons
     become available even if their own indexed catalog is missing and an
     earlier skeleton still resolves if a later record in the same file is
     truncated. Lookups fail loudly if the catalog is missing, fails before
     decoding the requested name, or lacks the name.
   - Each `Kind::Appearance` entry keeps its first registration (`0x59fe53`),
     and its referenced `.bcf` catalog is decoded once to confirm the
     catalog actually defines the named appearance (`0x5a044d`, `0x5a38da`,
     `0x5950f5`, `0x5a27f2`) before adding it to `suits` (decode errors are
     recorded in `suit_errors`).
   - `.ndx` stream failures are attributed by `IndexReader::failed_kind()`
     to the failing group: skeleton-group errors block later skeletons and
     record `suit_errors`; appearance-group errors record `suit_errors`
     only; animation-group errors affect neither.

Files outside `appearance` and `indexed` (such as `GameData/Objects` or
`GameData/UserObjects`) are no longer scanned. Both `People::load` /
`People::load_each` and `pool_skeletons` (`sims_vm::neighborhood_open`) now
use `AvatarRegistry`, and `People::load` resolves skeletons byte-for-byte
with first-registration precedence instead of failing on duplicates or
folding ASCII case.

## Byte-exact string decoding without BOM stripping

`sims_formats::strings::decode` previously called `Encoding::decode`, which
sniffs and strips UTF-8 (`EF BB BF`) and UTF-16 BOMs even when decoding a
Windows-1252 `STR#` chunk. It now calls `decode_without_bom_handling`,
matching `sims_formats::reader::western` and the original's raw byte
comparisons (`0x5a280f`).

## Unverified / deferred

- Text `.cmx` skeleton records (`0x59e746`) remain unimplemented and fail
  loudly when encountered (none exist in the Complete Collection).
- `People::suits` checks each `.ndx` appearance entry against its own
  first-registered `.bcf` catalog (`0x5a044d` cold lookup); it does not
  model cross-catalog side-effect population if a different suit's `.bcf`
  happened to be opened earlier at runtime (all 341 skeleton and appearance
  entries across `Animation.ndx` and `Downloads` resolve in their own
  catalogs in the Complete Collection).
