# Opening a neighborhood fills five pools after the Patch import

*Decided 2026-11-20.*

Opening a template neighborhood does more than house 0's reload and the
`Patch` import (`20261119-neighborhood-open-imports-patch-families.md`).
The original then fills five pools of houseless people. Track 2's
snapshot of its first UserData2 open (`ud2-import-20261119`, finished copy
`after-with-lot-load`) shows all five. The partial copy shows the first
three. Sol's research, from the original's code and both copies, is the
source here. Its notes and probes are kept outside the repository (track
4's `research/district-imports`, `NOTES.md` and `VACATION.md`).

## The five pools

| Order | Pool | Families | People | Called from | Gate | Template (`TemplateNPCs`) |
|---|---|---|---:|---|---|---|
| 1 | Studio Town celebrities | 5000 | 12 | the neighborhood update, 0x5580a2 / 0x5580b1 → 0x42c532, 0x42c1ea | Superstar 0x40 | `SSCelebGeneric.iff` (kind 0x800) |
| 2 | Studio Town fans | 5500 | 18 | 0x5580be / 0x5580cd → 0x42d4db, 0x42d193 | Superstar 0x40 | `HDT_Generic.iff` (kind 0x200) |
| 3 | Magic Town residents | 6000 | 30 | 0x5580da / 0x5580e9 → 0x42d92b, 0x42d5f2 | Makin' Magic 0x80 | `MMClownieGeneric.iff` (kind 0x1000) |
| 4 | Hot Date Townies | 2000 | 30 | the view's lot and toolbar callbacks (six sites, for example 0x55855d / 0x55856c) → 0x42aff1, 0x42a98f | Hot Date 0x04 | the 36 `HDT_*_*.iff` specialties (kind 0x400), else `HDT_Generic.iff` |
| 5 | Vacation Island tourists | 3000–3009 | ten families of 2–4 (35 here, 34 in UserData) | the same callbacks, straight after (for example 0x558579 / 0x558588) → 0x42b8c0, 0x42b0fd | Vacation 0x08 | `HDT_Generic.iff` |

- **Order of calls.** Each pool has a one-time wrapper and then a fill worker. The neighborhood setup update calls only pools 1–3; the other view's update repeats them (0x55fb05–0x55fb4c). Pools 4 and 5 run only from the six lot and toolbar callbacks (0x55855d–0x558588, 0x558757–0x558782, 0x5591e3–0x55920e and, in the other view, 0x56002a–0x560055, 0x560205–0x560230, 0x56082e–0x560859). The lot callbacks run them before the lot load (0x4f1ae9). Reopening the view alone does not run them. Which callback ran them in the recorded open is not traced.
- **Flags and top-up.**
  - Each pool has a pending/done pair in NGBH #1, stored from file versions 66 (Townies), 68 (tourists), 70 and 73. The reader loads each byte on its own (0x4254f6). Pools 1–3's wrappers test their done byte. The Townie and tourist wrappers test their pending byte (0x42b007, 0x42b8d7).
  - Each worker of pools 1–4 fills its family up to its target, so a later run tops up a pool that has lost members.
  - The tourist worker is different. It makes whichever of the ten families 3000–3009 are missing and leaves a family that exists, whatever its size.
- **Families.** A missing family is made with the reserved ID, house 0, budget 0 and flags 0. Members are appended in creation order (0x410849) after each person is saved.
- **People.**
  - Every pool person, like every `Patch` person, is cloned by 0x4722b8, through its own front end. `Patch` enters at 0x4260e1 and finds its template through the records. Pools 1–3 enter at 0x42cc96. The Townies and tourists enter at 0x426348 and 0x426bc0. All of them share the neighborhood's file counter, `+0x160`.
  - The OBJD kind becomes 2. Each record takes the last empty slot or a new one (0x4257a5 with ID −1): mode 9, revision 10, word 61 = 0.
  - Each person is edited through temporary people (0x4ea400; 0x4ea95f replaces the first): one for a Townie, two for everyone else, children included.
- **Tourist families.** Each family's size comes from two draws on the pool generator, `R(1000) % 100`:
  - adults: 0–59 gives a man and a woman, 60–79 a man, 80–99 a woman;
  - children: 0–24 and 70–84 give one, otherwise two.

  So a family has 2 to 4 people. Its surname comes from `Live.iff` STR# 160. Its first names come from STR# 161–164.

## Random streams

1. **A pool generator** (0x5bc82b, draws at 0x5bc882), seeded from `timeGetTime`. It picks templates, tone, body, head, names, the celebrities' rank and words, and the tourists' family make-up.
2. **The C runtime stream.** Each person draws one GUID low word (0x4725ff). Then each of its temporary people takes a skeleton copy (1,392 draws, adult or child) and two default-trait passes (0x459a9c). Sol's replay of this schedule fits every pool person in both neighborhoods: every GUID low word and all 15 saved trait words.

   | UserData2 pool | Draws |
   |---|---:|
   | Celebrities, fans and Magic Town | 176,409 |
   | 64 draws whose caller is unknown; then the Townies | 44,096 |
   | No draw between; the tourists | 102,893 |

3. **The game's own stream,** through the templates' scripts. These draws are not measured.

## The snapshot, explained

UserData2 before held 2 families and 49 record slots. After, it held 22 families, 446 slots and 153 new character files:

| Pool | Files | Records |
|---|---|---|
| `Patch` | User00002–29 | 294–321 |
| Celebrities | User00030–41 | 322–333 |
| Fans | User00042–59 | 334–351 |
| Magic Town | User00060–89 | 352–381 |
| Townies | User00090–119 | 382–411 |
| Tourists | User00120–154 | 412–446 |

- In the finished copy, word 61 of the first three pools' 60 records holds their family IDs, as a reload's rebuild writes them (0x424e89 → 0x425244). The 65 later records keep 0. So a rebuild ran after the Magic Town pool and before the Townies' families were filled, but when exactly is not traced.
- Only the five houses the `Patch` import moved families into changed.

The first neighborhood (UserData) holds the same five pools, already full, as its maker's open left them (User00048–171). Its User00013–15, 17 and 20–23 are older clones of TemplatePerson. They belong to no family, and all eight are in `Tutorial.FAM`'s records. None of the imports above makes them; what made them is not known.

## What is ported, and when

- **Now (6a).** `an_imported_home_loads_from_the_originals_files_and_steps` boots UserData2's House 69 from the original's own after-import files (`20261120-ud2-house-69-boots-from-the-originals-import.md`).
- **6b part E.** A port of the five pools in memory, after 6a. It covers the families, records, GUIDs and the C runtime schedule, fed with the recorded clock and pool-generator inputs. Its check is an equal strict-boot state against the original's after-import files. Sol estimates 735–1,250 lines.
- **Deferred, open.** The pool people's exact files and looks: the appearance picks, the scripts' draws on the game stream, the portraits and the names. These need an instrumented recording. Sol estimates 600–1,200 more lines.

**Unverified:**
- the callback that ran the Townie and tourist pools in the recorded open, when the rebuild before them ran (a reload also resets the file counter), and the 64 C runtime draws between the Magic Town pool and the first Townie;
- the C runtime state before the `Patch` import;
- the game stream's draws inside the templates' scripts;
- the pool generator's state across its reseeds;
- the one-time wrappers' path when a family with a reserved ID already exists.
