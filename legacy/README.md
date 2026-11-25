# Legacy prototype (reference only)

This directory holds the first prototype, frozen at the `legacy-final` tag. It
is **not** part of the Cargo workspace, isn't built or tested, and its
internal paths and dependencies are no longer maintained.

The prototype tried to mirror the original executable's internals: native
addresses, captured memory, emulated Win32 services, and resumable copies of
native call graphs. The vision (`docs/vision.md`) drops that goal. The code
still encodes hard-won knowledge of primitive semantics, file formats,
routing and rendering, so it stays here to be read while the new crates are
written.

## Rules

- **Never edit legacy code.** Read it, port from it, cite it.
- **A whole crate may be deleted** once a mining audit shows nothing in it is
  left to port: every piece of knowledge is either ported (with its new
  location) or deliberately dropped (with the vision, ADR or salvage-audit
  line that drops it). If in doubt, it stays. See
  `docs/decisions/20261123-legacy-crate-by-crate-cleanup.md`.
- **Everything stays recoverable** from the tag `legacy-final` (e1b72647). A
  citation of a deleted crate is written `legacy-final:crates/<crate>/...`. The
  tag predates the move under `legacy/`, so `legacy/crates/<crate>/<path>` here
  is `crates/<crate>/<path>` there. Read it with
  `git show legacy-final:crates/<crate>/<path>`.

## Crates

`docs/salvage-audit.md` gives the original port/reference/drop verdicts.
`docs/legacy-mining-audit.md` gives the per-crate mining audit and the
backlog of what is still to mine.

| Crate | State |
| --- | --- |
| sims-app | stays (still to mine: see the mining audit) |
| sims-content | stays (still to mine) |
| sims-render | stays (still to mine) |
| sims-sim | stays (still to mine) |
| sims-vm | stays (still to mine) |
| sims-fixtures | removed 2026-11-23; at `legacy-final` |
| sims-kernel | removed 2026-11-23; at `legacy-final` |
| sims-tools | removed 2026-11-23; at `legacy-final` |
| sims-formats (leftovers) | removed 2026-11-24; at `legacy-final` |
| sims-runtime | removed 2026-11-25; at `legacy-final` |

The remaining crates' `Cargo.toml` files still name the removed crates as
dependencies. That doesn't matter, because nothing here is built.

The suite manifests of the removed `sims-runtime` conformance runner,
`fixtures/suites/`, were removed on 2026-11-25 because nothing read them.
They are at `legacy-final:fixtures/suites/`.
