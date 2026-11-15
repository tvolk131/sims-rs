# Archive reads are bounded; compressed FAR members are left out (unverified)

*Decided 2026-11-14.*

Archive readers in `sims-content` and the census now cap what they load,
with the frozen prototype's limits: a FAR directory of at most 64 MiB
(`far::MAX_DIRECTORY`) and a resource of at most 128 MiB
(`far::MAX_MEMBER`). A member is checked against its archive again when
it is read, because the file may have changed since indexing. The
census opens archives inside archives at most four deep, rejects an
archive whose member overlaps its header or directory (such a member can
be the archive itself), and stops decoding members of one file on disk
beyond four times its length plus 16 MiB; it reads an archive on disk a
member at a time. An archive member whose name is longer than 1024 bytes
or holds more than 32 directories is left out of the index with a
diagnostic, since each directory in a member's name is one the index
lists. The game's own files are far inside all of these limits (member
names reach 96 bytes and four directories), and the install census is
unchanged.

A FAR 1a member whose unpacked size differs from its stored size is
compressed. Decompression is not implemented, so such a member is left
out of the index with a diagnostic, as the prototype refused it, and the
rest of its archive mounts. In `GameData/Animation`, which holds only the
game's own files and is read outside the index, such a member fails the
load instead, so a clip or skeleton never goes missing silently.
**Unverified**: whether the original lists
such a member, and so whether it hides a later file of the same name.
The game's own archives hold none. Keeping both sizes on
`sims_content::Member` waits on `sims-people`, which builds that struct
itself and also reads FAR directories without these caps.
