//! The game's sound: file decoders, the sound scripts and their player.
//!
//! The simulation never depends on this crate. It reads game files and
//! turns them into PCM (16-bit signed samples); playing them on a device
//! is left to a host binary. The `play` example is one, behind the
//! `playback` feature (off by default).
//!
//! - [`xa`]: Maxis XA, the 4-bit ADPCM that holds nearly every sound
//!   effect and voice.
//! - [`wav`]: RIFF WAVE with PCM samples.
//! - [`mp3`]: MPEG-1 Layer III, the radio stations' music.
//! - [`files`]: every sound file of an install, loose or in a FAR
//!   archive, by the path the sound tables name it with.
//! - [`decode`]: picks the decoder from a file's first bytes.
//! - [`hit`], [`hsm`], [`hot`]: the sound scripts. A HIT program is
//!   bytecode; its HSM file names the program's symbols and its HOT file
//!   maps event names to tracks, tracks to entry points and patches
//!   (samples), and lists the hit lists scripts pick samples from.
//! - [`module`]: one HIT program with its HSM and HOT files.
//! - [`fsc`]: ambient sound scores.
//! - [`ambience`]: the player of those scores, as the original plays
//!   them.
//! - [`levels`]: volume, pan and pitch, from registers to gains and a
//!   playback rate.
//! - [`stations`]: the radio, television and music hit lists the
//!   original builds from directories.
//! - [`player`]: the HIT virtual machine and a mixer. It takes
//!   [`SoundEvent`]s, the simulation's only link to sound, and renders
//!   PCM.

pub mod ambience;
pub mod decode;
mod error;
mod event;
pub mod files;
pub mod fsc;
pub mod hit;
pub mod hot;
pub mod hsm;
pub mod levels;
pub mod module;
pub mod mp3;
mod pcm;
pub mod player;
pub mod stations;
pub mod wav;
pub mod xa;

pub use error::Error;
pub use event::SoundEvent;
pub use pcm::Pcm;

pub type Result<T> = std::result::Result<T, Error>;
