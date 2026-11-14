//! What the simulation tells the sound system.
//!
//! The simulation's only link to sound is a stream of these events. It
//! never reads anything back, so sound cannot change what the simulation
//! does. The simulation crates do not emit them yet; the interface they
//! would need is in the track's report and `docs/decisions.md`.

/// One request from the simulation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SoundEvent {
    /// Start the HOT event of that name (such as `Phone_ring`) for an
    /// object. `object` is the simulation's object ID; scripts read it
    /// back through [`crate::player::Sources`].
    Play { object: u32, name: String },
    /// An object goes: what the game does with kind 23. Each track the
    /// object is recorded as playing for fades out and ends, unless
    /// another object still has it (see [`crate::player`]).
    StopObject { object: u32 },
    /// Stop every sound at once (kind 20).
    StopAll,
}
