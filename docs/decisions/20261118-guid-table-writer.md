# Share the house GUID table reader and writer

*Decided 2026-11-18.*

The family importer reads `Gtab` chunk 1 through 0x41578e at
0x429652, then appends its new GUID pairs at 0x429754. The port uses
`GuidTable::parse` and `GuidTable::translate`, the same code used
when loading a house. Both readers take the byte order from the first
byte and skip the version and tag. The existing pairs stay ahead of
the new pairs.

The import writes the combined table via 0x4157b6, which passes
version 0 to 0x415835. The stream writer explicitly clears all four
leading bytes at 0x464d40–0x464d48. It writes the version and the
`batG` tag at 0x464d4b–0x464d55, and clears the byte-swap flag at
0x464d6d before writing the 32-bit count and the old and new GUIDs
through 0x41588c and 0x4158a4. `GuidTable::to_bytes` always writes
this little-endian layout, including when the source table was
big-endian.

The import reports truncated tables with the native read addresses.
The original's handling of truncated tables is **unverified**.
