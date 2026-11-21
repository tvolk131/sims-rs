# The screenshot ratchet

*Decided 2026-11-20.*

The test `crates/sims-game/tests/ratchet.rs` boots the Goth house once,
runs it to each scene's tick, draws the frame at the scene's camera and
compares it pixel for pixel with a frozen screenshot of the original. The
scenes are in `crates/sims-game/tests/ratchet.txt`, which holds numbers and
hashes only. The screenshots are private, stay under `cache/ratchet/` and
never enter the repository.

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
differ from ours for reasons other than drawing, and the people class of
the 8:16 scenes measures that as well as the drawing.

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
- Our frame's SHA-256 is pinned. Any drawing change re-pins it with
  `SIMS_RATCHET_UPDATE=1`, and the PR body gives the new shares.
- A share below its floor fails. Update mode raises floors to the share
  reached and never lowers one; lowering a floor is a hand edit, with the
  reason in the PR body.
- `SIMS_RATCHET_DIFF=<folder>` writes difference images (pixels that
  differ in magenta, the rest ours in grey). The folder must be under
  `cache/`.
- `scripts/check_repository.py` now rejects image files, by suffix or by
  content, in any `tests`, `test`, `fixtures`, `testdata`, `test-data` or
  `test_data` folder.

## Running it

Without `SIMS_INSTALL` and `SIMS_RATCHET` the test prints `ratchet:
skipped: no assets` and passes, which is what CI sees. With them:

```
SIMS_INSTALL=<install> SIMS_RATCHET=<sims-rs>/cache/ratchet \
  cargo test --release -p sims-game --test ratchet
```

The coordinator's merge gate runs it beside the whole-day boot. A track
runs it before handing off any PR that touches drawing or people.

## What would reopen this

A class split finer than `id_at` allows, a capture of the original that
replays our run exactly (so people can be compared without the run's
drift), or a scene the ratchet should cover and does not.
