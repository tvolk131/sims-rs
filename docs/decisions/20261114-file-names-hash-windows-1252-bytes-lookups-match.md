# File names hash as Windows-1252 bytes; lookups match the last path component (unverified)

*Decided 2026-11-14.*

The index places an archive member by the bytes of its name as the
archive stores them, not by the name decoded to UTF-8: the original's
file table hashes those bytes, so a decoded `café.iff` landed in the
wrong bucket and could reverse load order, and so which GUID definition
wins. A loose file's name is encoded to Windows-1252 before it is hashed
(the code page a Western Windows hands the game), so a loose file and a
member of one name stay one entry; a name that is not valid UTF-8 or
has a character the code page lacks keeps the file system's bytes.
**Unverified**: the install holds no name outside ASCII, and a
non-Western code page would differ.

`Index::named` compares a resource's last path component, so a GLOB link
to `SofaGlobals.iff` finds an archive member `sub\SofaGlobals.iff`. A
name that holds a directory still matches the whole member name, as
before, and `Resource::name` keeps the whole name for reporting. Which of several
such files the original picks stays unverified, as before.

A string table whose leading word is not one of the four version words
(0, -1, -2, -3) is a Mac `STR#` resource with a big-endian count, even
when that word would be negative read as a version: 128 strings start
`00 80`.

On the install, resource order, semi-global links, the census and the
gated digests are unchanged.
