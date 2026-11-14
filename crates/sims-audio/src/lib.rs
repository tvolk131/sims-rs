//! The game's sound: file decoders and the sound scripts' tables.
//!
//! The simulation never depends on this crate. It reads game files and
//! turns them into PCM (16-bit signed samples); playing them on a device
//! is left to a host binary.
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

pub mod decode;
mod error;
pub mod files;
pub mod fsc;
pub mod hit;
pub mod hot;
pub mod hsm;
pub mod module;
pub mod mp3;
mod pcm;
pub mod wav;
pub mod xa;

pub use error::Error;
pub use pcm::Pcm;

pub type Result<T> = std::result::Result<T, Error>;
