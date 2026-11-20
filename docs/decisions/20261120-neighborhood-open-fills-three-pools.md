# Opening a neighborhood fills three pools after the Patch import

*Decided 2026-11-20.*

Opening a template neighborhood does more than house 0's reload and the
`Patch` import (`20261119-neighborhood-open-imports-patch-families.md`).
The same view update then fills three pools of houseless people. Track
2's snapshot of the original opening UserData2 for the first time
(`ud2-import-20261119`) shows them. Sol's research, from the original's
code and that snapshot, is the source here. Its notes and probes are kept
outside the repository (track 4's `research/district-imports`).

## What the original does

The neighborhood update (0x557fd3) runs the `Patch` import (0x558077 →
0x42c1d5 → 0x42c0ff). Then it calls each pool's one-time wrapper and,
straight after it, that pool's fill worker:

| Order | Pool | Family | Target | Calls | Expansion bit | Template (`TemplateNPCs`) | Kind |
|---|---|---:|---:|---|---|---|---|
| 1 | Studio Town celebrities | 5000 | 12 | 0x5580a2, 0x5580b1 → 0x42c532, 0x42c1ea | Superstar 0x40 | `SSCelebGeneric.iff` | 0x800 |
| 2 | Studio Town fans | 5500 | 18 | 0x5580be, 0x5580cd → 0x42d4db, 0x42d193 | Superstar 0x40 | `HDT_Generic.iff` | 0x200 |
| 3 | Magic Town residents | 6000 | 30 | 0x5580da, 0x5580e9 → 0x42d92b, 0x42d5f2 | Makin' Magic 0x80 | `MMClownieGeneric.iff` | 0x1000 |

- **Gates and top-up.**
  - The wrappers check NGBH #1's option pairs at +0x1a4, +0x1a6 and +0x1a8, which the file stores from versions 70 and 73 (0x4254f6).
  - Each worker fills its family from its current member count up to the target. A full pool returns on the count check, so a later open tops up a pool that has lost members.
  - The other view update (0x55faac) makes the same six calls (0x55fb05–0x55fb4c).
- **Progress boxes.** The boxes the original shows come from `GameData/UIText.iff` STR# 271 and 272: Superstar for the celebrities, Superstar for the fans, and "Abracadabra!" for Magic Town. The "Unleashed" box is the `Patch` import's (STR# 270).
- **Families.**
  - A missing family is made with the reserved ID, house 0, budget 0 and flags 0 (0x41003c). It is named from STR# 271 row 1, 271 row 10 or 272 row 10.
  - Each new person's GUID is appended (0x410849) after that person is saved, so a pool record's word 61 stays 0 until the next reload rebuilds it (0x424e89 → 0x425244).
- **People.** Each is cloned from the template's IFF by the clone `Patch` uses (0x42cc96 → 0x4722b8, with a GUID from 0x4725ff). The files share the neighborhood's file counter with `Patch`. Its OBJD kind becomes 2 (0x42cf09).
  - Its record takes the last empty slot, or a new one (0x4257a5 with ID −1): mode 9, revision 10.
  - Two temporary people are made to edit it (0x4ea400, then 0x4ea95f). Its body, head, name and portraits are written (0x4e995d, 0x4ea538, 0x4e8c5d).
- **Three random streams.**
  1. A pool generator (0x5bc82b) seeded from `timeGetTime`. It chooses tone, the first name, body and head, and the celebrities' rank and six words.
  2. The C runtime stream. Each person takes:
     - one draw for the GUID;
     - for each of the two temporary people, a skeleton copy (1,392 draws) and two default-trait passes (0x459a9c).

     This schedule fits every saved pool member in both neighborhoods. That covers all 60 GUIDs' low words and all 900 saved interest and personality values in each. UserData2's pools take 35,278, 52,958 and 88,173 draws.
  3. The game's own stream, through the templates' scripts (Init and their own BHAVs). These draws are not measured.

## The snapshot, explained

UserData2 before had 2 families and 49 record slots. After it had 11 families, 381 slots and 88 new character files:
- `Patch`: User00002–29 (records 294–321);
- celebrities: User00030–41 (322–333);
- fans: User00042–59 (334–351);
- Magic Town: User00060–89 (352–381).

The reload made 270 records, 26 of them in empty slots. Only the five houses the `Patch` import moved families into changed.

The first neighborhood (UserData) holds the same three pools, already full, as its maker's open left them (User00048–107). Its User00013–15, 17 and 20–23 are older clones of TemplatePerson. They are in no family, and all eight are in `Tutorial.FAM`'s records. No import makes them.

## What is ported, and when

- **Now (6a).** `an_imported_home_loads_from_the_originals_files_and_steps` boots UserData2's House 69 from the original's own after-import files, laid over the install's UserData2
  (`20261120-ud2-house-69-boots-from-the-originals-import.md`).
- **6b part E.** A port of the pools in memory, after 6a:
  - the families, records, GUIDs and the C runtime schedule;
  - fed with the recorded clock and pool-generator inputs.

  Its check is an equal strict-boot state against the original's after-import files. Sol estimates 500–850 lines.
- **Deferred, open.** The pool people's exact files and looks: the appearance picks, the scripts' draws on the game stream, the portraits and names. These need an instrumented recording. Sol estimates 400–800 more lines.

**Unverified:**
- the C runtime state before the `Patch` import, and how it carries into the first pool GUID;
- the game stream's draws inside the templates' scripts;
- the one-time wrappers' path when a family with the reserved ID already exists. Their scan reads one past the end of the family list (0x42c5e4–0x42c5ec).
