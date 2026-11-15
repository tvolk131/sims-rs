# Decisions

Choices made while working toward `docs/roadmap.md`. Each choice is its own
file in this folder, so two pull requests never edit the same lines.

## Writing an entry

- Name the file `YYYYMMDD-short-slug.md`: the date it was decided, then a
  few lowercase words from the title joined by hyphens. Add `-2`, `-3` if
  the name is already taken.
- Start with the title as a `#` heading, then `*Decided YYYY-MM-DD.*`.
- Say what was decided, why, and what would reopen it. Mark a guess that
  waits on the comparison harness **unverified**.
- Add a new file rather than editing an older entry. When a later choice
  replaces an earlier one, say so in the new entry and name the old file.
  Fixing a mistake in an entry is fine.

There is no index to update. `ls docs/decisions` lists entries by date, and
`grep -l` finds them by subject.
