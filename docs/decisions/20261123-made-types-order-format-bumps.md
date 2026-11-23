# Older builds refuse stores that keep made types in the order made

*Decided 2026-11-23.*

The original gives each person type made in play (Make New Character) a
place in its table of person types in the order it was made, and that
place breaks the relationship panel's ties. Two types made in one tick can
have descending GUIDs, so the GUID does not give the order.
`Characters::made` now keeps the order made (`MadeTypes`). It still encodes
as a map, now in the order made.

An older build keeps made types by GUID. If it could read one of the new
encodings, it would save the types back in GUID order without any warning.

What was decided:

- **Older builds must refuse, not reorder.** Every stored encoding that
  carries the made types gets a new version that an older build rejects:
  - the neighborhood store: `NEIGHBORHOOD_FORMAT` goes from 4 to 5;
  - the save records: `AUTOSAVE_FORMAT` goes from 2 to 3 (both manual saves
    and autosaves, for every payload kind);
  - the named snapshots: each record's version word goes from 32 to 33.
    `snapshot::FORMAT`, the version of the world's encoding, stays at 32,
    since `Saved` writes and checks it too and bumping it would make every
    older `Saved` refuse;
  - the trip envelope (`TRVL`, the active trip kept in place of the home
    lot while a household is away): its version goes from 2 to 3, for
    the made types in its departure baseline.

  Raw world bytes (`World::encode`, `World::encode_snapshot`) carry no
  version of their own and are protected only by the wrappers above.

  `Saved` and the co-op protocol need no extra bump: this change already
  moved them (format 11, protocol 2), and main has neither.
- **The layouts are unchanged.** Each new version has the layout of the
  version before it, so the bump costs nothing but the version word.
- **Older versions still load, by GUID.** A format-4 neighborhood, a
  format-1 or format-2 save record (`LoadedSave::decode_world`, and the
  Save Manager's restore), a named snapshot record of version 32 and a
  trip envelope of version 1 or 2 all read their made types back in GUID
  order, as an older build wrote them.

What would reopen it: a need for an older build to open a new build's
saves, such as a downgrade path. The bumps would then need a downgrade
migration, not just a version.
