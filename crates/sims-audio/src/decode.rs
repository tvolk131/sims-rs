//! Picks a decoder from a file's first bytes.

use crate::{Pcm, Result, mp3, wav, xa};

/// The sound formats the install holds.
///
/// UTK (the codec of The Sims Online) is not here: no file in the
/// Complete Collection install starts with `UTM0`, nor carries the
/// extension (checked by the census test).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Xa,
    Wav,
    Mp3,
}

impl Kind {
    /// The format of `bytes`, from its magic number. MP3 is recognized by
    /// an ID3 tag or an MPEG audio frame sync (eleven set bits).
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"XA") {
            Some(Self::Xa)
        } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            Some(Self::Wav)
        } else if bytes.starts_with(b"ID3")
            || (bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
        {
            Some(Self::Mp3)
        } else {
            None
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Xa => "XA",
            Self::Wav => "WAV",
            Self::Mp3 => "MP3",
        }
    }

    pub fn decode(self, bytes: &[u8]) -> Result<Pcm> {
        match self {
            Self::Xa => xa::decode(bytes),
            Self::Wav => wav::decode(bytes),
            Self::Mp3 => mp3::decode(bytes),
        }
    }
}

/// Decodes `bytes` with the decoder its magic number names, or returns
/// None for a file that is not sound.
pub fn decode(bytes: &[u8]) -> Option<Result<Pcm>> {
    Kind::sniff(bytes).map(|kind| kind.decode(bytes))
}
