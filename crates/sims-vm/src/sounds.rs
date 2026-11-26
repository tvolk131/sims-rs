//! Sound events: what the simulation tells the sound system.
//!
//! The original hands sound requests to its sound manager (the object at
//! [0x6e48bc]+0x10) and never reads anything back that the simulation
//! keeps, so sound cannot change what the lot does. The interpreter
//! notes each request as a [`SoundEvent`] in a sink ([`Interpreter::sounds`])
//! instead; a lot update returns them ([`crate::Update::sounds`]). They are
//! not part of the world, so they do not change its hash.
//!
//! Requests noted, read from the original's code:
//!
//! - Play Sound Event (opcode 23, 0x452822) plays the sound its FWAV chunk
//!   names ([`SoundEvent::Play`], 0x57a54c).
//! - Go To Routing Slot plays "begin_route" as a route starts (step 12,
//!   0x45b989) and "end_route" after the animator stops a route (0x45b3a0,
//!   which steps 9, 10 and 11 reach: the route arrived, stopped short or
//!   failed). Step 15 stops the animator without it (0x45b3aa), and so
//!   does the end of a route (steps 7 and 8, 0x45b6ea, which goes straight
//!   to the primitive's exit at 0x45b952). Steps 11 and 15 are not ported.
//!   The steps are read from the state table at 0x45b9d4.
//! - Stop All Sounds (opcode 48, 0x45256e), an object's reset (0x431fbe
//!   at 0x4320b4, also when it is being deleted) and an object's
//!   destruction (0x4323c9) stop the object's sounds ([`SoundEvent::Stop`],
//!   0x57a6fb). So a deleted object's sounds are stopped twice. A
//!   person's reset (0x45e791) does not call it.
//!
//! - A clip's timed events "sound", "selectedsound", "deselectedsound"
//!   and "footstep" (0x591505 to 0x59175e) play the sound they pick from
//!   where the person stands, its selection, or the floor under it
//!   ([`crate::footsteps`]; `clip_events.rs`).
//!
//! Play Sound Event's interface branch (0x4a698d, when [0x6e48bc]+0x5b is
//! set and I am a person) is interface, not sound, and is not modeled.
//!
//! [`Interpreter::sounds`]: crate::Interpreter::sounds

/// One request to the sound system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoundEvent {
    /// Play the sound event of that name for an object (0x57a54c with the
    /// name and the object's ID). The ID is what the script named, as the
    /// original passes it: it is not checked to name an object.
    Play { object: i16, name: String },
    /// Stop the object's sounds (0x57a6fb).
    Stop { object: i16 },
}
