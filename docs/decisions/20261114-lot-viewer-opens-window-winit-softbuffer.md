# The lot viewer opens a window with winit and softbuffer

*Decided 2026-11-14.*

`crates/sims-viewer` is a small program that shows a lot drawn by
`sims-render` in a window. It opens the window with `winit` and copies
the CPU-drawn frame into it with `softbuffer`; no GPU is involved. These
two crates are dependencies of `sims-viewer` only. `sims-render` stays
free of window code, so tests draw offscreen and `cargo test` never
needs a display, and `sims-vm`, `sims-sim` and `sims-core` never see
either crate.

The frame is drawn at the original's pixel size and scaled up by whole
pixels on high-DPI screens, so pixels stay sharp. The keys follow the
original where it has one: three wall modes, quarter-turn rotations,
three zooms and two floors. The viewer reads either a house file or a
world saved by `sims boot --snapshot`, and only reads it.

The two crates are the same versions the people viewer uses (see the
entry below), so `Cargo.lock` gains no new packages. Unlike the people
viewer, `sims-viewer` is not behind a feature: it is its own crate, and
CI builds it. The Linux X11 and Wayland back ends load their system
libraries at run time, so CI needs no new system packages.

What would reopen it: the UI track needing a GPU, text or widgets that
softbuffer cannot give, at which point the window code moves to wgpu
or another toolkit, still outside `sims-render`.
