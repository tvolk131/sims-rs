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

`docs/salvage-audit.md` lists, crate by crate, what to port, what to read
for behavior only, and what to leave. Each legacy crate is deleted once it has
been fully mined. The tag keeps everything recoverable.
