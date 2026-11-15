# Placeholder objects carry no registry word

*Decided 2026-11-13.*

Every ObjM record ended with a field-encoded registry word, until the
expansion houses: the `*Placeholder` object (GUID 0x3C08965F) never has one.
Across 139,000 ordinary records in the installed and shipped houses, 24
to 31 bits remain after the dynamic sprites. Across 27,000 placeholder
records, 0 to 7 remain, so no field fits. The reader now reads the word only
when at least a byte of the record remains, and stores None otherwise.
Why the original writes no word is unknown; it may skip objects missing
from the scheduler's registry. Unverified until the harness compares a
placeholder.
