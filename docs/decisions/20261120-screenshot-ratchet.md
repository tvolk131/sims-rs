# The screenshot ratchet

*Decided 2026-11-20.*

The test `crates/sims-game/tests/ratchet.rs` boots the Goth house once,
runs it to each scene's tick, draws the frame at the scene's camera and
compares it pixel for pixel with a frozen screenshot of the original. The
scenes are in `crates/sims-game/tests/ratchet.txt`: each scene's settings,
a line about what it shows, the screenshot's file name and SHA-256, our
frame's SHA-256, the rectangles left out with their reasons, and the
floors. No pixels are in it. The screenshots are private, stay under
`cache/ratchet/` and never enter the repository.

## Scenes

Eight scenes, all with the original paused and the panel's strip closed:

| Scene | Time | Zoom | Rotation | Walls | Shows |
|---|---|---|---|---|---|
| `near-r2-0712-bench` | 7:12 AM | near | 2 | down | Mortimer reading on the bench |
| `near-r2-0805` | 8:05 AM | near | 2 | down | Mortimer by the stairs, Bella on the sofa |
| `far-r0-cut-0816` | 8:16 AM | far | 0 | cutaway | the whole lot, the school bus and the carpool |
| `far-r1-cut-0816` | 8:16 AM | far | 1 | cutaway | the whole lot, turned once |
| `medium-r1-cut-0816` | 8:16 AM | medium | 1 | cutaway | the porch and a Sim on the lawn |
| `near-r3-down-0816` | 8:16 AM | near | 3 | down | indoors: the kitchen and the bathroom |
| `near-r0-up-0816` | 8:16 AM | near | 0 | up | the house's side, windows and the stairs |
| `night-far-r0-cut-2153` | 9:53 PM | far | 0 | cutaway | the lot at night, the house left out |

Each camera origin is the original's scroll position less one in x; every
8:16 render aligned with its screenshot at a shift of (0, 0).

The 7:12 and 8:05 screenshots are the ones the people comparison already
used (#282). The others were taken for the ratchet from one run of the
original: paused at 8:16:18 AM, then run on at the fastest speed to the
night scene.

The live run is not the run our game replays. On the original, the Help
dialog's No click also opened a "Go Here" pie menu on the lot. It stayed
listed from 6:54 until a click on the lot dismissed it at about 8:16, and
the run's notices were answered by hand (two about going downtown and a
skill point, among others). So where a Sim is, and what it does, can
differ from ours for reasons other than drawing.

The people floors of the 8:16 scenes (0.74% to 1.60%) are agreement
between two different runs, driven mostly by where each Sim stands and
how it is posed; they say little about how people are drawn. Judging the
drawing of people needs a capture of the original synchronised with our
run. The 7:12 and 8:05 people floors (17.16% and 14.13%) come from the
#282 comparison, where the Sims stand where ours do.

By 9:53 PM the live run had had a kitchen fire that ours does not have.
The night scene leaves the house out and measures the terrain, trees,
hedges, gravestones and road under the night's light. A near night scene
was taken and dropped: the fire filled most of it.

## Classes

Classes come from the frame at run time, with no stored masks:

- `panel`: pixels over the control panel;
- `people`: pixels a person drew last (`Canvas::id_at` names a person);
- `objects`: pixels any other object drew last;
- `surface`: the rest. Walls, floors, terrain, grass and the road are all
  drawn by nobody, so they are one class;
- `frame`: every pixel not left out.

`id_at` cannot tell walls from floors or grass, and it cannot separate
lighting or shadows from what they fall on. Splitting `surface` needs the
canvas to record which pass drew each pixel; that is a follow-up, not part
of this change. A class with no pixels in a frame keeps no floor.

A pixel matches when both colours agree in 16-bit colour (5, 6 and 5
bits), the depth the original draws in. No class is exact in any scene
yet, so every floor is the share reached today, rounded down to a
hundredth of a percent.

## What is left out

Each scene names its rectangles and why:

- the original's red "Paused" notice, which only the screenshot has;
- the panel's pause button, which blinks while paused;
- the plumb bob, which spins;
- the fish tank, whose fish swim;
- the fountain, whose water runs;
- in the night scene, the house (the fire) and the tab over the lot,
  which shows what the run's selected Sim is doing.

## Rules

- A screenshot whose SHA-256 differs from the manifest stops the run. A
  new capture needs a new hash and the reason in its PR.
- Our frame's SHA-256 is pinned over the whole frame, the regions left
  out included. That is deliberate: our drawing is deterministic, so the
  pin is the check that nothing we draw changed, a fish in our tank
  included. The regions left out hide only the original's animations, and
  only from the shares. Any drawing change re-pins it with
  `SIMS_RATCHET_UPDATE=1`, and the PR body gives the new shares.
- A share below its floor fails. Floors are written with exactly two
  decimals, at most 100.00 (at most three whole digits, read before any
  arithmetic), and compared as whole hundredths of a percent.
- A class with pixels must have a floor, a class with none must not, and a
  scene that leaves every pixel out fails.
- Update mode (`SIMS_RATCHET_UPDATE=1`) re-pins hashes and raises floors
  to the share reached. It never adds, lowers or removes a floor. A new
  scene is written with both `ours = none` and `floors = none`, and
  `SIMS_RATCHET_NEW=<name>` sets its hash and first floors; that is the
  only way a floor is added. Once they are set, the scene's name joins
  the baseline list in the test. A baseline scene written as new, or
  named in `SIMS_RATCHET_NEW`, stops the run, so its floors can never be
  set anew.
  Lowering or removing a floor is a hand edit, with the reason in the PR
  body.
- A run that fails writes nothing. A write replaces the manifest whole (a
  new file, then a rename), and only if the manifest's SHA-256 is still
  the one the run read; there is no lock between processes. Run one
  updater at a time: the hash check catches edits made between read and
  write, not two updaters racing.
- The manifest is strict: an unknown or repeated setting, a setting
  before the first scene, a missing baseline scene (the eight above, named
  in the test), a bad floor, or a rectangle that is not inside the frame
  stops the run.
- A screenshot is named by a plain file name, with no folder. With links
  resolved, it must be a file in `SIMS_RATCHET`, so the manifest cannot
  point the run at any other file. The reference hash is computed over
  the bytes read, so a swapped file fails; the corpus is assumed not to
  be hostile.
- `SIMS_RATCHET_DIFF=<folder>` writes difference images (pixels that
  differ in magenta, the rest ours in grey) into a new folder the run
  makes inside it, each image a new file. The folder, with links
  resolved, must be under a folder named `cache` and outside the checkout,
  its own `cache/` included.
- The test reads its arguments once, as libtest does: a flag that takes a
  value takes the next word (`--skip --list` skips `--list`), and every
  word after `--` is a filter. Filters, `--exact`, `--skip`, `--ignored`
  and `--include-ignored` select as libtest's do. Arguments libtest
  refuses fail before any filter or list applies: an unknown flag, a flag
  without its value (a bare `--skip`), a value on a flag that takes none
  (`--list=foo`), a bad value, and `--ignored` with `--include-ignored`.
- Before anything else the test checks its own floor and argument
  parsing, so the release run in the merge gate covers them.
- `scripts/check_repository.py` now rejects image files anywhere in the
  repository: by suffix, or by content (raster formats; netpbm headers,
  comments ended by a line feed or a carriage return; an SVG root element
  under any namespace prefix). The SVG root is found with Python's expat
  parser, which reads the whole prolog, an internal document type with
  `]>` in its quoted values included, stops at the first start element,
  and loads no external entity. Each file is read twice and either
  reading that finds an `svg` root counts: as bytes in the encoding the
  document declares (a Latin-1 prefix such as `·` is read as Latin-1),
  and as text decoded by the byte order mark or else as UTF-8 with bad
  bytes replaced (so a declared encoding expat cannot read, such as
  Shift_JIS, or an unknown one, does not hide an SVG).

## Running it

Without `SIMS_RATCHET` the test prints `ratchet: skipped: no assets` and
passes. That is what CI sees, and what a track's workspace gate sees: it
sets only `SIMS_INSTALL`, for the other tests. Every other partial or
wrong setting fails, on the skip too, and before `--help`, `--list` or a
filter ends the run: `SIMS_INSTALL` set but empty, or not a folder
holding a regular file at some depth; `SIMS_RATCHET` without
`SIMS_INSTALL`, or not such a folder; any `SIMS_RATCHET_` variable,
whatever its value (`SIMS_RATCHET_UPDATE=0` included), without
`SIMS_RATCHET`, its name matched as bytes so that one that is not text
counts; a `SIMS_RATCHET_` variable that is not one of the three
settings; `SIMS_RATCHET_UPDATE` other than 1 or 0; `SIMS_RATCHET_NEW`
naming a baseline scene, a scene with floors or no scene; and
`SIMS_RATCHET_DIFF` that is not an existing folder allowed to hold
difference images. Every one of these is checked in one function that
runs before the arguments are read, so no way out of the test skips a
check.
With both:

```
SIMS_INSTALL=<install> SIMS_RATCHET=<sims-rs>/cache/ratchet \
  cargo test --release -p sims-game --test ratchet
```

The coordinator's merge gate runs this command beside the whole-day boot
and fails unless it prints `ratchet: ok`. A track runs it before handing
off any PR that touches drawing or people.

## What would reopen this

A class split finer than `id_at` allows, a capture of the original that
replays our run exactly (so people can be compared without the run's
drift), or a scene the ratchet should cover and does not.
