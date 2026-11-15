# Directory order is NTFS name order (unverified)

*Decided 2026-11-13.*

Superseded by "The catalog follows the original's scan order and file
tables" above: NTFS name order now decides only the order of names that
share a bucket.

The original lists each mounted directory with FindFirstFile, and that order
decides the object catalog order. On Windows/NTFS it is name order with case
folded. `Index::build` uses that order and searches a subdirectory where its
name falls among the files. Neither detail has been confirmed against the
original's walker yet. The old harness ran the game under Wine on ext4,
whose directory order can differ from NTFS. The comparison harness must
either record the order the original saw and feed it to us, or put the
install on a file system that lists in NTFS order.
