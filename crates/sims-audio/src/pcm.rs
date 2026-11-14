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

    /// A RIFF WAVE file holding the samples, for listening by hand.
    pub fn to_wav(&self) -> Vec<u8> {
        let data = self.bytes();
        let block = u32::from(self.channels) * 2;
        let mut out = Vec::with_capacity(44 + data.len());
        out.extend(b"RIFF");
        out.extend((36 + data.len() as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(self.channels.to_le_bytes());
        out.extend(self.rate.to_le_bytes());
        out.extend((self.rate * block).to_le_bytes());
        out.extend((block as u16).to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }
}
