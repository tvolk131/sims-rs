# The state hash is SHA-256 of the postcard encoding

*Decided 2026-11-13.*

`World` is encoded with postcard: fields in declaration order, integers as
little-endian varints, floats as raw bits (NaN payloads and negative zero
kept), and maps in key order. Nothing in it depends on the platform, and a
pinned-hash test runs on both CI platforms. Snapshots store the same bytes
behind a 4-byte format number, in a REDB table keyed by name. Changing the
encoding means bumping `snapshot::FORMAT` and the pinned hash. Comparison
with the original goes field by field, not through this hash: the hash only
proves that our own runs repeat.

`World` keeps every field the readers decode for objects, the simulator
and the lot arrays. It leaves out bytes nobody understands yet (format
tails, raw ObjM records), the HOUS view and model, and the wall and floor
chunks. They join when the interpreter or the comparison needs them.
