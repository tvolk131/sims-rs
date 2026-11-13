# sims-rs

An unofficial, open-source Rust engine for the original **The Sims** (2000)
and its seven expansion packs. It runs the game's own scripts on a faithful,
deterministic reimplementation of the simulation, using game files that
players supply themselves, with a modern macOS front end and co-op
multiplayer. See [the vision](docs/vision.md) for the full scope.

**No original game, executable, assets, dialogue, fonts, screenshots or player
saves are included.** Any game content needed to exercise the engine must be
obtained separately, used with the necessary rights, and kept private.

## Status

The project is being re-founded. An earlier prototype mirrored the original
executable's internals. That code now lives under [legacy/](legacy/README.md)
as reference material and is not built. The
[salvage audit](docs/salvage-audit.md) records what is being carried forward.
The [roadmap](docs/roadmap.md) sets the current goal: a headless lot boot that
matches the original game tick for tick.

| Crate | Responsibility |
| --- | --- |
| `sims-core` | Deterministic primitives: the x87-faithful float model and the game's random streams. |
| `sims-formats` | Readers for the game's archive, resource and save formats. |
| `sims-content` | Finds a game install and indexes its files in the original's load order. |
| `sims-cli` | The `sims` developer tool, starting with a content census. |

## Determinism rules

Simulation code must give identical results on every machine. Clippy enforces
the mechanical part (see `clippy.toml`): no platform math functions, no
wall-clock reads, no hash-ordered collections. Floats in simulation state use
`sims_core::F32`, and arithmetic uses `sims_core::Ext`.

## Build and test

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
python3 -B scripts/check_repository.py
```

CI runs these on x86_64 Linux and Apple Silicon macOS.

## Licensing and provenance

Project-contributed source and documentation use the [MIT license](LICENSE).
Dependencies and consulted upstream projects have separate terms; see
[third-party notices](THIRD_PARTY_NOTICES.md) and the
[dependency license inventory](docs/dependency-licenses.md).

Read [licensing and publication boundaries](docs/legal-and-provenance.md) before
contributing or distributing builds. Development used original-engine analysis
and open-source format references: **this is not a clean-room claim**. The MIT
license does not grant rights to Electronic Arts/Maxis content, upstream code or
trademarks, and an engineering audit is not legal clearance.

The initial public commit is a new, source-only history. Earlier local history,
research notes, capture scripts, native fixtures and private backups are not
published. See the [repository workflow](docs/repository-workflow.md) for upload
gates and contribution requirements. Please do not attach original-game data,
captures or screenshots to issues or pull requests.
