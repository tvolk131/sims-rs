# The people viewer is a feature-gated window

*Decided 2026-11-14.*

`people-viewer` is a small window binary in `sims-people`. It uses
`winit` (the window and its events) and `softbuffer` (copies the
software rasterizer's image to the window). Both are optional
dependencies behind the crate's `viewer` feature, and nothing else in
the workspace enables it. So the default build, `cargo test` and CI
never compile them. They add 187 packages to `Cargo.lock`, which CI does
not download, and leave every existing package's version unchanged. The
cost is that CI does not build the viewer; run
`cargo clippy -p sims-people --features viewer --all-targets` after
changing it.

The viewer reads the wall clock to play clips in real time, so the
binary allows the workspace's determinism lint. Nothing it computes
reaches the simulation. It reads a simulation snapshot through
`sims_sim::Snapshots` and makes its own `Animator` values for the
standing and walking demonstration; it never changes simulation code.

`--png <file>` draws one frame without a window, so the viewer's
drawing can be checked headless.
