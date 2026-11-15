# The catalog follows the original's scan order and file tables

*Decided 2026-11-13.*

Objects register in the order the original scans for them (0x43c609):
`GameData/Objects`, `GameData/UserObjects`, the neighborhood's
`Characters`, `Downloads`, the packs in bit order up to Superstar, the
shared directories, and Makin' Magic last. `Index::add_characters` puts
the people in that place. This replaces "characters register after the
install" below.

Within a directory the original does not use the file system's order. It
keeps each directory's entries in a hash table keyed by lower-case name and
walks the table (0x5ba9bd). `sims-content`'s `file_table` reproduces the
hash, the bucket counts and the growth rule, ported from the old prototype.
An archive's members are entries of the directory the archive is in, and a
member whose name holds a path goes into that subdirectory, whether or not
it exists on disk. The first entry of a name wins, except that a file on
disk replaces an archive member. This replaces "directory order is NTFS
name order" below.

It is checked against the original. When the Complete Collection loads a
lot, it creates the lot-wide controllers the lot lacks by walking its
catalog. A capture from the old prototype's harness holds the 141 objects
that made, in order. Picking the same definitions from our catalog gives
the same 141 GUIDs in the same order. The test
`the_catalog_lists_controllers_in_the_originals_order` pins a digest of
that order and runs when `SIMS_INSTALL` is set. The capture itself is
private.

What that leaves **unverified**:

- The order of two names that share a bucket, and which of two archives
  supplies a member both hold. Both depend on the order Windows lists a
  directory in, for which `Index` uses NTFS name order. The 141 controllers
  show no such case.
- Where the people fall. No controller is a person, so their place rests on
  the old prototype's reading of the scan.
- Houses saved by the original list their object types in catalog order,
  and those lists disagree with the capture in one way: they show larger
  tables for some directories (389 buckets for the base game's objects,
  not 193). The studio's machines presumably had more files in those
  directories. The capture is from the install we support, so it wins.
