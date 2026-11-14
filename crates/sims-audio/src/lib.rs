//! The game's sound: file decoders and, later, the sound scripts.
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

pub mod decode;
mod error;
pub mod files;
pub mod mp3;
mod pcm;
pub mod wav;
pub mod xa;

pub use error::Error;
pub use pcm::Pcm;

pub type Result<T> = std::result::Result<T, Error>;
