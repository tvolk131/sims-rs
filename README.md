# sims-rs

An unofficial, experimental Rust compatibility engine and set of inspection tools
for the original **The Sims** game formats and behavior. This is a work in
progress, not a replacement game distribution or a claim of complete gameplay,
rendering or save compatibility.

**No original game, executable, assets, dialogue, fonts, screenshots or player
saves are included.** Any game content needed to exercise the engine must be
obtained separately, used with the necessary rights, and kept private.

## Project layout

| Crate | Responsibility |
| --- | --- |
| `sims-formats` | Archive, resource and binary-format readers. |
| `sims-content` | Installation/resource lookup, content loading and save adapters. |
| `sims-vm` | Script VM, continuation state and bounded compatibility services. |
| `sims-sim` | World, object, routing, animation and simulation behavior. |
| `sims-runtime` | Runtime/session orchestration and comparison tools. |
| `sims-render` | Software/GPU rendering and visual state. |
| `sims-app` | Experimental room viewer and archive inspector. |
| `sims-tools` | Command-line inspection and development tools. |
| `sims-fixtures` | Content-addressed local evidence storage and integrity checks. |

The implementation contains explicit partial/unsupported paths. A decoded file,
a passing component test or a saved JSON snapshot does not establish complete
original-engine parity or an original-game-readable save. Keep backups and do
not test against the only copy of a player's installation or saves.

## Build and public tests

The current CI toolchain is Rust 1.99.0. Start with:

```sh
cargo check --locked --workspace --lib --bins --tests
cargo test --locked --lib -p sims-formats -p sims-vm -p sims-sim \
  -p sims-content -p sims-runtime -p sims-fixtures -- --test-threads=1
python3 -B -m unittest discover -s scripts -p 'test_*.py'
```

GPU/application builds also require the native window-system dependencies for
your platform. Linux builds explicitly enable Iced's Wayland backend without
enabling its unrelated default features; running the GUI requires a Wayland
session. On Ubuntu, CI installs `libwayland-dev`, `libxkbcommon-dev` and
`libegl1-mesa-dev`. Runtime tests that need a GPU, game content or a private
observation are separate from the asset-free library unit suites above.
Manual research examples are not part of that CI check; known pre-existing
example compile issues are recorded in the repository workflow.

See the [avatar-codec selector notes](docs/repository-workflow.md#avatar-codec-test-selectors)
for the consolidated synthetic integration-test target and exact case names.

Explore the CLI without opening a game or a GUI:

```sh
cargo run --locked -p sims-tools --bin sims -- --help
cargo run --locked -p sims-app --bin sims-room -- --help
cargo run --locked -p sims-fixtures -- --help
```

### Private evidence is opt-in

Captured reference corpora and tests containing original-game bytes or prose
are deliberately absent from the public repository. Their dependent tests are
gated behind `private-fixtures`; some otherwise synthetic tests share those
private test modules. Do not use `--all-features` as a self-contained public-clone
check. The feature does not download evidence or grant redistribution rights.

A maintainer with the separately retained local inputs can deliberately run, for
example, `cargo test --locked -p sims-vm --features private-fixtures --lib`.
A public clone cannot reproduce every private comparison, and no extra parity
coverage is claimed merely by omitting those comparisons. New public test data
should be genuinely synthetic, with its provenance explained.

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
