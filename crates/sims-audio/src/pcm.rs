use crate::{Error, Result};

/// Decoded sound: interleaved 16-bit signed samples.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pcm {
    /// Frames per second.
    pub rate: u32,
    /// Samples per frame: 1 (mono) or 2 (stereo, left first).
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Pcm {
    /// The number of frames (one sample per channel).
    pub fn frames(&self) -> usize {
        match self.channels {
            0 => 0,
            n => self.samples.len() / usize::from(n),
        }
    }

    /// The samples as little-endian bytes, the form digests are taken of.
    pub fn bytes(&self) -> Vec<u8> {
        self.samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    /// A RIFF WAVE file holding the samples, for listening by hand. An
    /// error when a size does not fit the format's 32 bits, or the
    /// channel count is not 1 or 2.
    pub fn to_wav(&self) -> Result<Vec<u8>> {
        let fail = |message: &str| Err(Error::format("WAV", 0, message));
        if !matches!(self.channels, 1 | 2) {
            return fail("only mono and stereo are written");
        }
        let block = u32::from(self.channels) * 2;
        let Some(byte_rate) = self.rate.checked_mul(block) else {
            return fail("byte rate does not fit 32 bits");
        };
        let Some(size) = self
            .samples
            .len()
            .checked_mul(2)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|&n| n <= u32::MAX - 36)
        else {
            return fail("data does not fit 32 bits");
        };
        let mut out = Vec::with_capacity(44 + size as usize);
        out.extend(b"RIFF");
        out.extend((36 + size).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(self.channels.to_le_bytes());
        out.extend(self.rate.to_le_bytes());
        out.extend(byte_rate.to_le_bytes());
        out.extend((block as u16).to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend(size.to_le_bytes());
        out.extend(self.bytes());
        Ok(out)
    }
}
