# Patch family import returns neighborhood changes and the lot's GUID table

*Decided 2026-11-18.*

The data step takes parsed neighborhood state and one parsed FAM. It
returns the new types and neighbor slots, family records, and the lot
resources the caller can use at neighborhood open. The catalog and
neighbor slots are the state after the original has registered people at
`0x424e72` and assigned family membership at `0x425244`. For example,
the raw UserData2 NBRS lacks the dog and cat templates. The
neighborhood's registration pass gives them records before import.

## Characters and neighbors

`0x428f86` reads EXPi #0. The member loop at `0x42926b–0x429635`
takes the nonzero IDs before the first zero, up to eight. Each ID
selects a uChr resource and a one-based slot in the exported NBRS
vector. At `0x4292cf–0x429359` the game looks at uChr string **13**
(one-based), the gender string. The comparison at `0x636490` searches
for the case-sensitive substrings `dog` and then `cat`. This also
recognizes shipped cats whose skeleton string reads `kat`.
TemplatePerson is `0x7fd96b54`, TemplateDog is `0x4a70df92` and
TemplateCat is `0x7bea0977`. The dog and cat branches require the
`0x20` bit at game options +0x66.

`0x4260e1` looks up the template's first neighbor record and uses its
definition's IFF. Its character-file counter (neighborhood +0x160)
starts at zero after neighborhood load (`0x424e8e`). It attempts
`Characters/User%05d.iff` at the current number. An existing file
returns -0x2b before any GUID generation (`0x4722b8`). While copying an
available file, `0x47249d` invokes `0x4725d5` once per copied OBJD in
the file's resource enumeration order. The old template GUID goes to
OBJD byte 0x8c and the generated GUID to 0x1c. At `0x4725ff` the clock
is read first, followed by one CRT draw at `0x636c74`:

```text
state = state * 0x343fd + 0x269ec3 (mod 2^32)
crt_rand15 = (state >> 16) & 0x7fff
guid = ((timeGetTime() & 0xffff) << 16) ^ crt_rand15
```

After each copy attempt the counter increases once. At 32000 it resets
to zero. If that last attempt found an occupied file, the current
clone fails and the search does not resume at zero. The next member
starts at the resulting counter. The copy is registered at `0x43c33d`
before recording its first unrepresented person definition
(`0x43ac3a`, `0x425770`, `0x4257a5`). The pure import supports
one OBJD per template and calls its GUID source in EXPi order. The SIMI
read before cloning at `0x4290e2` restores the separate simulator
global RNG (`0x46cfb3`, `0x46f44d`). With the neighborhood open and
no active lot, the zoning lookup at `0x40745f` takes the residential
path. The nonresidential path would read the clock twice for temporary
lot and global seeds. Neither path uses the CRT draw above.

The new type uses the template with the imported body text in STR# 200
(`0x4293ae–0x429444`). The writer uses a new string table populated
from the selected uChr strings. The name and description come from
CTSS at exported ID + 2000 (`0x42945d`) or the same ID through
`0x4061e3` (`0x429516`). The game sets the new definition's label
to its file stem, a space, hyphen and space, and the name
(`0x429596–0x4295f7`). The type result preserves the text and file
number from which that label is derived.

`0x4257a5` tries the requested EXPi slot first. A slot beyond the
vector extends it with null entries. If the requested slot is occupied,
the last vacant slot is used, or a slot is appended. The fresh record
at `0x4237a1` has the new ID and GUID, the new file stem for its name,
and empty relationships and zeroed person storage. After all copies,
`0x429799–0x4297d5` takes the source's person revision and copies
only the words in `0x423827` whose required revision is met. The rest
keep their fresh zero values. `0x425244` finally writes the destination
family ID into person word 61. The result expresses the fresh record
as the current NBRS layout (10 with the additional version word 9,
`0x423e08`), with the full in-memory person storage.

For each new record, `0x429759–0x429799` visits the EXPi IDs in order.
`0x429bf1` reads the first relationship row for each exported target
ID, then copies all its values to the corresponding new target ID.
Absent and empty rows produce no destination row. Relationships to
people outside the export are omitted. This translation uses neighbor
IDs. Family membership uses GUIDs.

## Family and lot

`0x425dea` initially allocates the first free positive family ID and
the greatest existing family number plus one, with a minimum number
of 1 before overflow. `0x429126–0x42915a` then keeps the exported ID
if it is free. FAMI's member vector is built by appending the made
GUIDs in EXPi order (`0x429380–0x429388`). The import takes the
exported budget and architecture value (`0x4291c2–0x4291d8`), and
sets only the exported bit 0 (`0x4298b1–0x4298c3`). The secondary
house and magic words start at zero. `0x427bd7` recomputes the
friend count at `0x42986c`. The imported members' relationships
only link one another, so they have no reciprocal friends outside
the family at this point. The FAMs name comes from the exported
family, overridden by CTSS #1000 when present (`0x428ff3–0x42901f`).
The neighborhood writer stores the selected name in FAMs in the
current language, writes FAMI version 9, and transfers the optional
FAMh payload under the assigned ID (`0x4291de–0x4291f6`,
`0x410696–0x410807`).

The house number is FAMI's primary house (`0x4291ec`), with zero
meaning no lot. For a nonzero number the importer first clears the old
occupant's primary house and architecture value and clears its flag
bit 3 (`0x4291ff–0x429235`). It tries to remove character records whose
person word 67 equals the lot number (`0x429251`, `0x426fcd`).
Records whose GUID belongs to any family remain. The displaced
family's budget is retained at neighborhood
open, when no lot is active (`0x4272c5`). The placement helper
`0x42731e` requires residential zoning and the cleared slot to be
free before assigning it to the new family. The temporary move-in
flag it sets is cleared at `0x4298aa`. If SIMI global 26 is zero
or SIMI is absent, the importer clears neighborhood word 2 when it
equals the exported house number (`0x4298dd–0x4298ef`).

`0x429652` reads Gtab #1 if present. `0x42972e–0x429754` appends
the old NBRS GUID and the made GUID as a pair, in EXPi order.
`0x4297f7` writes the resulting Gtab #1 to a housed export.
On lot load `0x43ddc2` reads the table and `0x43a6fb` applies
it before looking up each objt GUID. `0x415751` restarts at the
first pair after each substitution and uses each pair at most once
per GUID lookup. The import returns the live lot resources with this
Gtab and the parsed objt table with its GUIDs resolved. ObjM references
type IDs, which the import preserves.

`0x4299a6` constructs the destination `Houses/House%02d.iff` name.
The importer moves the existing house to its `.tmp` name and moves the
patched FAM into the house's place (`0x4299db–0x429a06`). Failure of
that second move attempts to restore the old house (`0x429a08`).
After success it deletes the temporary house. With no lot, it deletes
the FAM itself (`0x429a75–0x429a82`, `DeleteFileA` at
`0x66624c`). In both cases it then saves the neighborhood at
`0x429a92`. The loop from `0x42bca4` finds the Patch files and
calls through `0x4e2bf5` for import. The imports at `0x42bf94`
and `0x42bfa6` are GetFileAttributesA and SetFileAttributesA.
The intervening bytes in the captured image include breakpoints,
so the exact wrapper behavior there remains unverified.

## Scope and unresolved paths

The result is a data change without disk or UI calls. The original's
SIMI version-69 follow-up (`0x429ad5–0x429b1a`) and the wrapper after
`0x4e2c0c` regenerate character portraits via `0x4e8c5d`. That
routine saves BMP_ resources through `0x4e9000`. Portrait generation,
its runtime state and any RNG it consumes remain outside the pure
import. The meaning of the opaque FAMh fields remains unverified.

The API returns address-bearing errors for the disabled-pets drop
(`0x42601e`), FINV inventory transfer (`0x42a92e`), legacy CATS
character text (`0x406222`), the nonzero SIMI global 26 type-attribute
reset (`0x429903`), and cleanup of displaced records with no family
(`0x426fcd`). It also reports unsupported template ambiguity or
multiple OBJD copies, generated GUID collisions, and malformed or
ambiguous member and lot inputs. In the original the nonzero SIMI
global 26 branch sets neighborhood words 2 and 3 to the lot and zero
and clears per-type attributes through `0x455f5f`. The port signals
that branch before returning an incomplete result.
