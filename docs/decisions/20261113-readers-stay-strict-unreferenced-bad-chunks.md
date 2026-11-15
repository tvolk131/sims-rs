# Readers stay strict on unreferenced bad chunks

*Decided 2026-11-13.*

21 TTAB chunks in the original install are version 3 or zero bytes long.
Nothing references them. The reader keeps rejecting them, and loaders
decode only the chunks they use. Reopen if a referenced chunk ever fails.
